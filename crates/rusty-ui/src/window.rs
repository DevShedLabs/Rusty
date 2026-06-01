use std::sync::Arc;

use fontdue::{Font, FontSettings};
use rusty_config::Config;
use rusty_core::{Color, Grid};
use rusty_hint::HintEngine;
use rusty_render::{RenderDoc, RenderTrigger, detect_trigger, trigger::{strip_trailing_prompt, strip_trailing_prompt_json}};
use rusty_mux::pane::Pane;
use rusty_pty::{Pty, PtySize};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, KeyEvent, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{Window, WindowAttributes, WindowId},
};
#[cfg(target_os = "macos")]
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS, WindowAttributesExtMacOS};

const FONT_SIZE: f32 = 16.0;
const FONT_BYTES: &[u8] = include_bytes!("../assets/JetBrainsMono-Regular.ttf");

// ── cell measurement ─────────────────────────────────────────────────────────

fn measure_cell(font: &Font, size: f32) -> (usize, usize, usize) {
    let (m, _) = font.rasterize('M', size);
    let lm = font.horizontal_line_metrics(size).unwrap();
    let cell_w = if m.advance_width > 0.0 { m.advance_width.ceil() as usize } else { (size * 0.6).ceil() as usize };
    let line_h = (lm.ascent - lm.descent + lm.line_gap).ceil();
    let cell_h = if line_h > 0.0 { line_h as usize } else { (size * 1.4).ceil() as usize };
    let baseline = lm.ascent.ceil() as usize;
    (cell_w, cell_h, baseline)
}

// ── bind group helper ─────────────────────────────────────────────────────────

fn make_bind_group(
    device:  &wgpu::Device,
    bgl:     &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    view:    &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label:   Some("blit_bg"),
        layout:  bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}

fn make_screen_tex(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label:           Some("screen"),
        size:            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count:    1,
        dimension:       wgpu::TextureDimension::D2,
        format:          wgpu::TextureFormat::Rgba8UnormSrgb,
        usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats:    &[],
    })
}

// ── GPU state (created after window is ready) ─────────────────────────────────

struct Gpu {
    device:       wgpu::Device,
    queue:        wgpu::Queue,
    surface:      wgpu::Surface<'static>,
    surface_cfg:  wgpu::SurfaceConfiguration,
    pipeline:     wgpu::RenderPipeline,
    bgl:          wgpu::BindGroupLayout,
    sampler:      wgpu::Sampler,
    screen_tex:   wgpu::Texture,
    bind_group:   wgpu::BindGroup,
    pub fb_w:     usize,
    pub fb_h:     usize,
    framebuf:     Vec<u8>,
}

impl Gpu {
    fn new(window: Arc<Window>) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..Default::default()
        });

        // SAFETY: window is Arc-owned and outlives the surface.
        let surface = instance.create_surface(window.clone()).expect("surface");

        // Use enumerate_adapters (sync) to avoid block_on deadlocking the macOS run loop.
        let adapter = instance
            .enumerate_adapters(wgpu::Backends::METAL)
            .into_iter()
            .find(|a| a.is_surface_supported(&surface))
            .expect("no Metal adapter supports this surface");

        // request_device is async but has no macOS run-loop dependency — safe to block
        // on a background thread and rendezvous with a channel.
        let (device, queue) = std::thread::scope(|s| {
            s.spawn(|| {
                pollster::block_on(adapter.request_device(
                    &wgpu::DeviceDescriptor {
                        label:             None,
                        required_features: wgpu::Features::empty(),
                        required_limits:   wgpu::Limits::default(),
                    },
                    None,
                )).expect("device")
            }).join().expect("device thread")
        });

        let caps          = surface.get_capabilities(&adapter);
        let surface_fmt   = caps.formats[0];
        let alpha_mode    = caps.alpha_modes[0];
        let phys          = window.inner_size();
        tracing::info!("surface fmt={surface_fmt:?} alpha={alpha_mode:?} phys={phys:?}");

        let surface_cfg = wgpu::SurfaceConfiguration {
            usage:        wgpu::TextureUsages::RENDER_ATTACHMENT,
            format:       surface_fmt,
            width:        phys.width.max(1),
            height:       phys.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_cfg);

        // ── pipeline ─────────────────────────────────────────────────────────
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter:     wgpu::FilterMode::Nearest,
            min_filter:     wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("blit_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled:   false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit_pl"), bind_group_layouts: &[&bgl], push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:    Some("blit"),
            layout:   Some(&pl),
            vertex:   wgpu::VertexState {
                module: &shader, entry_point: "vs_main", buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader, entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format:     surface_fmt,
                    blend:      None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive:     wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample:   wgpu::MultisampleState::default(),
            multiview:     None,
        });

        let fb_w = phys.width  as usize;
        let fb_h = phys.height as usize;
        let screen_tex  = make_screen_tex(&device, fb_w as u32, fb_h as u32);
        let screen_view = screen_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group  = make_bind_group(&device, &bgl, &sampler, &screen_view);
        let framebuf    = vec![0u8; fb_w * fb_h * 4];

        Self { device, queue, surface, surface_cfg, pipeline, bgl, sampler, screen_tex, bind_group, fb_w, fb_h, framebuf }
    }

    fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 { return; }
        self.surface_cfg.width  = w;
        self.surface_cfg.height = h;
        self.surface.configure(&self.device, &self.surface_cfg);
        self.fb_w = w as usize;
        self.fb_h = h as usize;
        self.framebuf   = vec![0u8; self.fb_w * self.fb_h * 4];
        self.screen_tex = make_screen_tex(&self.device, w, h);
        let view = self.screen_tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.bind_group = make_bind_group(&self.device, &self.bgl, &self.sampler, &view);
    }

    fn render(&mut self, pane: &Pane, font: &Font, font_px: f32, cell_w: usize, cell_h: usize, baseline: usize, top_inset: usize, selection: Option<Selection>, config: &Config, ghost: &str, popup: Option<&PopupState>, overlay: Option<(&RenderDoc, usize, Option<(usize,usize)>)>) {
        paint_framebuf(pane, font, font_px, cell_w, cell_h, baseline, top_inset, self.fb_w, self.fb_h, &mut self.framebuf, selection, config, ghost, popup, overlay);

        self.queue.write_texture(
            self.screen_tex.as_image_copy(),
            &self.framebuf,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row:  Some((self.fb_w * 4) as u32),
                rows_per_image: Some(self.fb_h as u32),
            },
            wgpu::Extent3d { width: self.fb_w as u32, height: self.fb_h as u32, depth_or_array_layers: 1 },
        );

        let frame = match self.surface.get_current_texture() {
            Ok(f)  => f,
            Err(e) => { tracing::warn!("frame err: {e}"); return; }
        };
        let fv  = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &fv, resolve_target: None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes:         None,
                occlusion_query_set:      None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(std::iter::once(enc.finish()));
        frame.present();
    }
}

// ── popup completion state ────────────────────────────────────────────────────

struct PopupState {
    entries:  Vec<rusty_hint::CompletionEntry>,
    selected: usize,
}

impl PopupState {
    fn new(entries: Vec<rusty_hint::CompletionEntry>) -> Option<Self> {
        if entries.is_empty() { return None; }
        Some(Self { entries, selected: 0 })
    }
    fn selected_insert(&self) -> &str {
        &self.entries[self.selected].insert
    }
    fn move_up(&mut self)   { if self.selected > 0 { self.selected -= 1; } }
    fn move_down(&mut self) { if self.selected + 1 < self.entries.len() { self.selected += 1; } }
}

// ── application ───────────────────────────────────────────────────────────────

/// A selected region in cell coordinates (col, row) relative to the scrollback view.
#[derive(Clone, Copy, Debug)]
struct Selection {
    start: (usize, usize),
    end:   (usize, usize),
}

impl Selection {
    /// Normalise so start ≤ end in reading order.
    fn normalised(&self) -> ((usize, usize), (usize, usize)) {
        let (sr, sc) = (self.start.1, self.start.0);
        let (er, ec) = (self.end.1,   self.end.0);
        if (sr, sc) <= (er, ec) {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        }
    }

    fn contains(&self, col: usize, row: usize) -> bool {
        let ((sc, sr), (ec, er)) = self.normalised();
        if row < sr || row > er { return false; }
        if row == sr && row == er { return col >= sc && col <= ec; }
        if row == sr { return col >= sc; }
        if row == er { return col <= ec; }
        true
    }
}

/// Logical pixels to reserve at the top of the window for the macOS traffic light buttons
/// when running with a transparent/hidden titlebar.
#[cfg(target_os = "macos")]
const TITLEBAR_INSET_LOGICAL: f64 = 28.0;
#[cfg(not(target_os = "macos"))]
const TITLEBAR_INSET_LOGICAL: f64 = 0.0;

struct App {
    shell:      String,
    config:     Config,
    font:       Font,
    gpu:        Option<Gpu>,
    window:     Option<Arc<Window>>,
    pty:        Option<Pty>,
    pane:       Option<Pane>,
    hint:       HintEngine,
    popup:      Option<PopupState>,
    pending_render:     Option<RenderTrigger>,
    capture_buf:        Vec<u8>,
    /// Instant of last PTY byte received while a render is pending.
    capture_last_byte:  Option<std::time::Instant>,
    overlay:            Option<(RenderDoc, usize)>,
    /// Selected row range within the overlay (start_row, end_row), inclusive.
    overlay_sel:        Option<(usize, usize)>,
    overlay_sel_start:  Option<usize>, // row where mouse-down started
    font_px:    f32,
    cell_w:     usize,
    cell_h:     usize,
    baseline:   usize,
    /// Physical pixels to skip at the top of the framebuffer (traffic light zone).
    top_inset:  usize,
    modifiers:  ModifiersState,
    selection:  Option<Selection>,
    selecting:  bool,
    cursor_pos: (f64, f64),
}

impl App {
    fn new(shell: &str, config: Config) -> Self {
        let font = Font::from_bytes(FONT_BYTES, FontSettings::default()).expect("font");
        let font_size = config.font.size;
        Self {
            shell:      shell.to_owned(),
            config,
            font,
            gpu:        None,
            window:     None,
            pty:        None,
            pane:       None,
            hint:       HintEngine::new(),
            popup:          None,
            pending_render:     None,
            capture_buf:        Vec::new(),
            capture_last_byte:  None,
            overlay:            None,
            overlay_sel:        None,
            overlay_sel_start:  None,
            font_px:    font_size,
            cell_w:     8,
            cell_h:     16,
            baseline:   13,
            top_inset:  0,
            modifiers:  ModifiersState::default(),
            selection:  None,
            selecting:  false,
            cursor_pos: (0.0, 0.0),
        }
    }

    fn pixel_to_cell(&self, x: f64, y: f64) -> (usize, usize) {
        let col = (x as usize / self.cell_w).min(self.pane.as_ref().map_or(0, |p| p.grid.width.saturating_sub(1)));
        let row = ((y as usize).saturating_sub(self.top_inset) / self.cell_h).min(self.pane.as_ref().map_or(0, |p| p.grid.height.saturating_sub(1)));
        (col, row)
    }

    fn selected_text(&self) -> Option<String> {
        let sel = self.selection?;
        let pane = self.pane.as_ref()?;
        let ((sc, sr), (ec, er)) = sel.normalised();
        let scroll_off = pane.scroll_off;
        let total      = pane.grid.total_rows();
        let view_start = total.saturating_sub(pane.grid.height).saturating_sub(scroll_off);

        let mut out = String::new();
        for row in sr..=er {
            let src_row = view_start + row;
            let col_start = if row == sr { sc } else { 0 };
            let col_end   = if row == er { ec } else { pane.grid.width.saturating_sub(1) };
            let mut line = String::new();
            for col in col_start..=col_end {
                let cell = pane.grid.scrollback_get(col, src_row);
                line.push(cell.ch);
            }
            // Trim trailing spaces from each line.
            let trimmed = line.trim_end_matches(' ');
            out.push_str(trimmed);
            if row < er { out.push('\n'); }
        }
        Some(out)
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attrs = {
            let base = WindowAttributes::default()
                .with_title("rusty")
                .with_inner_size(winit::dpi::LogicalSize::new(1024u32, 768u32));
            #[cfg(target_os = "macos")]
            let base = base
                .with_titlebar_transparent(true)
                .with_fullsize_content_view(true)
                .with_title_hidden(true)
                .with_movable_by_window_background(true);
            base
        };
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));

        let scale    = window.scale_factor() as f32;
        self.font_px = self.config.font.size * scale;
        (self.cell_w, self.cell_h, self.baseline) = measure_cell(&self.font, self.font_px);
        self.top_inset = (TITLEBAR_INSET_LOGICAL * window.scale_factor()).round() as usize;
        tracing::info!("scale={scale} font_px={} cell={}×{} baseline={} top_inset={}", self.font_px, self.cell_w, self.cell_h, self.baseline, self.top_inset);

        let phys = window.inner_size();
        let cols = (phys.width  as usize / self.cell_w).max(1);
        let rows = ((phys.height as usize).saturating_sub(self.top_inset) / self.cell_h).max(1);

        let pty = Pty::spawn(&self.shell, PtySize {
            cols: cols as u16, rows: rows as u16,
            px_w: phys.width as u16, px_h: phys.height as u16,
        }).expect("pty spawn");

        self.gpu    = Some(Gpu::new(window.clone()));
        self.pane   = Some(Pane::new(0, cols, rows));
        self.pty    = Some(pty);
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(phys) => {
                if phys.width == 0 || phys.height == 0 { return; }
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(phys.width, phys.height);
                }
                let cols = (phys.width  as usize / self.cell_w).max(1);
                let rows = ((phys.height as usize).saturating_sub(self.top_inset) / self.cell_h).max(1);
                if let Some(pane) = &mut self.pane {
                    pane.resize(cols, rows);
                }
                if let Some(pty) = &self.pty {
                    let _ = pty.resize(PtySize {
                        cols: cols as u16, rows: rows as u16,
                        px_w: phys.width as u16, px_h: phys.height as u16,
                    });
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y)   => y as f32,
                    winit::event::MouseScrollDelta::PixelDelta(pos)   => pos.y as f32 / self.cell_h as f32,
                };
                // Overlay scroll takes priority.
                if let Some((doc, scroll)) = &mut self.overlay {
                    let max_scroll = doc.lines.len().saturating_sub(1);
                    if lines > 0.0 { *scroll = scroll.saturating_sub(lines.ceil() as usize); }
                    else           { *scroll = (*scroll + (-lines).ceil() as usize).min(max_scroll); }
                    return;
                }
                if let Some(pane) = &mut self.pane {
                    if lines > 0.0 {
                        pane.scroll_up_view(lines.ceil() as usize);
                    } else {
                        pane.scroll_down_view((-lines).ceil() as usize);
                    }
                }
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }


            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = (position.x, position.y);
                let fb_h = self.gpu.as_ref().map(|g| g.fb_h).unwrap_or(1);
                let scroll_zone = self.cell_h * 2; // 2-row trigger zone at edges

                if self.overlay.is_some() {
                    if self.overlay_sel_start.is_some() {
                        let screen_row = ((position.y as usize).saturating_sub(self.top_inset) / self.cell_h).saturating_sub(1);
                        if let Some(start) = self.overlay_sel_start {
                            let lo = start.min(screen_row);
                            let hi = start.max(screen_row);
                            self.overlay_sel = Some((lo, hi));
                        }
                        // Auto-scroll overlay when dragging near top/bottom edge.
                        if let Some((doc, scroll)) = &mut self.overlay {
                            let max_scroll = doc.lines.len().saturating_sub(1);
                            if position.y as usize <= scroll_zone + self.cell_h {
                                *scroll = scroll.saturating_sub(1);
                            } else if position.y as usize >= fb_h.saturating_sub(scroll_zone) {
                                *scroll = (*scroll + 1).min(max_scroll);
                            }
                        }
                    }
                } else if self.selecting {
                    let cell = self.pixel_to_cell(position.x, position.y);
                    if let Some(sel) = &mut self.selection {
                        sel.end = cell;
                    }
                    // Auto-scroll terminal when dragging near top/bottom edge.
                    if let Some(pane) = &mut self.pane {
                        if position.y as usize <= scroll_zone {
                            pane.scroll_up_view(1);
                        } else if position.y as usize >= fb_h.saturating_sub(scroll_zone) {
                            pane.scroll_down_view(1);
                        }
                    }
                }
            }

            WindowEvent::MouseInput { state, button: winit::event::MouseButton::Left, .. } => {
                if self.overlay.is_some() {
                    match state {
                        ElementState::Pressed => {
                            let screen_row = ((self.cursor_pos.1 as usize).saturating_sub(self.top_inset) / self.cell_h).saturating_sub(1);
                            self.overlay_sel_start = Some(screen_row);
                            self.overlay_sel = Some((screen_row, screen_row));
                        }
                        ElementState::Released => {
                            self.overlay_sel_start = None;
                        }
                    }
                } else {
                    match state {
                        ElementState::Pressed => {
                            self.selecting = true;
                            self.selection = None;
                            let cell = self.pixel_to_cell(self.cursor_pos.0, self.cursor_pos.1);
                            self.selection = Some(Selection { start: cell, end: cell });
                        }
                        ElementState::Released => {
                            self.selecting = false;
                            if let Some(sel) = &self.selection {
                                if sel.start == sel.end {
                                    self.selection = None;
                                }
                            }
                        }
                    }
                }
            }

            WindowEvent::KeyboardInput {
                event: KeyEvent { logical_key, text, state: ElementState::Pressed, .. }, ..
            } => {
                // Overlay takes priority — handle scroll/dismiss before shell input.
                if let Some((doc, scroll)) = &mut self.overlay {
                    let max_scroll = doc.lines.len().saturating_sub(1);
                    let cmd = self.modifiers.super_key();
                    match &logical_key {
                        Key::Named(NamedKey::Escape) |
                        Key::Named(NamedKey::Enter) => {
                            self.overlay = None;
                            self.overlay_sel = None;
                            return;
                        }
                        Key::Character(s) if s.as_str() == "q" => {
                            self.overlay = None;
                            self.overlay_sel = None;
                            return;
                        }
                        Key::Character(s) if s.as_str() == "c" && cmd => {
                            // Cmd+C — copy selected overlay text, or whole doc if no selection.
                            let text = overlay_selected_text(doc, *scroll, self.overlay_sel);
                            copy_to_clipboard(&text);
                            return;
                        }
                        Key::Named(NamedKey::ArrowUp)    => { *scroll = scroll.saturating_sub(1); return; }
                        Key::Named(NamedKey::ArrowDown)  => { *scroll = (*scroll + 1).min(max_scroll); return; }
                        Key::Named(NamedKey::PageUp)     => { *scroll = scroll.saturating_sub(10); return; }
                        Key::Named(NamedKey::PageDown)   => { *scroll = (*scroll + 10).min(max_scroll); return; }
                        // Modifier-only keypresses — don't dismiss.
                        Key::Named(
                            NamedKey::Super | NamedKey::Shift | NamedKey::Control | NamedKey::Alt |
                            NamedKey::CapsLock | NamedKey::Meta
                        ) => { return; }
                        // Any other key dismisses.
                        _ => { self.overlay = None; self.overlay_sel = None; }
                    }
                }

                let ctrl = self.modifiers.control_key();
                let cmd  = self.modifiers.super_key(); // Cmd on macOS

                // Cmd+C — copy selection.
                if cmd {
                    if let Key::Character(s) = &logical_key {
                        match s.as_str() {
                            "c" => {
                                if let Some(text) = self.selected_text() {
                                    copy_to_clipboard(&text);
                                }
                                return;
                            }
                            "v" => {
                                if let Some(text) = paste_from_clipboard() {
                                    if let Some(pane) = &mut self.pane { pane.scroll_off = 0; }
                                    if let Some(pty)  = &mut self.pty  { let _ = pty.write_bytes(text.as_bytes()); }
                                }
                                return;
                            }
                            _ => {}
                        }
                    }
                }

                let bytes: Option<Vec<u8>> = if ctrl {
                    // Ctrl+key → control byte. Use logical_key so layout doesn't matter.
                    match &logical_key {
                        Key::Character(s) => {
                            let ch = s.as_str().chars().next().unwrap_or('\0').to_ascii_uppercase();
                            if ('A'..='Z').contains(&ch) {
                                Some(vec![ch as u8 - b'A' + 1])
                            } else {
                                match ch {
                                    '[' => Some(b"\x1b".to_vec()),  // Ctrl-[ = ESC
                                    '\\' => Some(b"\x1c".to_vec()),
                                    ']' => Some(b"\x1d".to_vec()),
                                    '^' => Some(b"\x1e".to_vec()),
                                    '_' => Some(b"\x1f".to_vec()),
                                    ' ' => Some(b"\x00".to_vec()),  // Ctrl-Space = NUL
                                    _ => None,
                                }
                            }
                        }
                        _ => None,
                    }
                } else {
                    // ── Popup navigation (takes priority over everything else) ──
                    if let Some(popup) = &mut self.popup {
                        match &logical_key {
                            Key::Named(NamedKey::ArrowUp) => { popup.move_up(); return; }
                            Key::Named(NamedKey::ArrowDown) => { popup.move_down(); return; }
                            Key::Named(NamedKey::Escape) => { self.popup = None; return; }
                            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Tab) => {
                                // Accept selected entry.
                                let insert = popup.selected_insert().to_owned();
                                self.popup = None;
                                // Send only the suffix after what's already typed.
                                let suffix = insert.strip_prefix(self.hint.line.as_str())
                                    .unwrap_or(&insert)
                                    .to_owned();
                                self.hint.line = insert;
                                if let Some(pane) = &mut self.pane { pane.scroll_off = 0; }
                                if let Some(pty)  = &mut self.pty  { let _ = pty.write_bytes(suffix.as_bytes()); }
                                return;
                            }
                            _ => {
                                // Any other key closes the popup and passes through normally.
                                self.popup = None;
                            }
                        }
                    }

                    // Tab — show popup (or accept ghost hint if no completions).
                    if matches!(&logical_key, Key::Named(NamedKey::Tab)) {
                        let entries = self.hint.completions();
                        if !entries.is_empty() {
                            self.popup = PopupState::new(entries);
                            return;
                        }
                        if let Some(suffix) = self.hint.accept_ghost() {
                            if let Some(pane) = &mut self.pane { pane.scroll_off = 0; }
                            if let Some(pty)  = &mut self.pty  { let _ = pty.write_bytes(suffix.as_bytes()); }
                            return;
                        }
                        // Nothing to complete — pass \t through.
                    }

                    // ArrowRight — accept ghost hint if showing, otherwise normal cursor move.
                    if matches!(&logical_key, Key::Named(NamedKey::ArrowRight)) {
                        if let Some(suffix) = self.hint.accept_ghost() {
                            if let Some(pane) = &mut self.pane { pane.scroll_off = 0; }
                            if let Some(pty)  = &mut self.pty  { let _ = pty.write_bytes(suffix.as_bytes()); }
                            return;
                        }
                    }

                    // Use the OS-composed text first, fall through to named key sequences.
                    if let Some(t) = &text {
                        let s = t.as_str();
                        // Update line buffer for hint tracking.
                        match s {
                            "\r" | "\n" => {
                                // Read the current input line directly from the grid —
                                // this works for typed input AND history recalled with ↑.
                                let grid_line = self.pane.as_ref()
                                    .map(|p| read_cursor_row(&p.grid, p.cursor.row))
                                    .unwrap_or_default();
                                let check_line = if grid_line.trim().is_empty() {
                                    &self.hint.line
                                } else {
                                    &grid_line
                                };
                                if let Some(trigger) = detect_trigger(check_line) {
                                    self.pending_render    = Some(trigger);
                                    self.capture_buf       = Vec::new();
                                    self.capture_last_byte = None;
                                    self.overlay           = None;
                                }
                                self.hint.commit();
                            }
                            "\x7f" => {
                                // Backspace
                                let mut line = self.hint.line.clone();
                                line.pop();
                                self.hint.update_line(&line);
                            }
                            "\x1b" => {
                                // Escape clears the hint line.
                                self.hint.update_line("");
                            }
                            _ => {
                                let mut line = self.hint.line.clone();
                                line.push_str(s);
                                self.hint.update_line(&line);
                            }
                        }
                        self.popup = None; // typing closes the popup
                        let bytes = s.as_bytes().to_vec();
                        if let Some(pane) = &mut self.pane { pane.scroll_off = 0; }
                        if let Some(pty)  = &mut self.pty  { let _ = pty.write_bytes(&bytes); }
                        return;
                    }

                    // Named keys with no text representation.
                    let bytes: Option<Vec<u8>> = match &logical_key {
                        Key::Named(NamedKey::Enter)        => {
                            let grid_line = self.pane.as_ref()
                                .map(|p| read_cursor_row(&p.grid, p.cursor.row))
                                .unwrap_or_default();
                            let check_line = if grid_line.trim().is_empty() { self.hint.line.clone() } else { grid_line };
                            if let Some(trigger) = detect_trigger(&check_line) {
                                self.pending_render    = Some(trigger);
                                self.capture_buf       = Vec::new();
                                self.capture_last_byte = None;
                                self.overlay           = None;
                            }
                            self.hint.commit();
                            Some(b"\r".to_vec())
                        }
                        Key::Named(NamedKey::Backspace)    => {
                            let mut line = self.hint.line.clone();
                            line.pop();
                            self.hint.update_line(&line);
                            Some(b"\x7f".to_vec())
                        }
                        Key::Named(NamedKey::Escape)       => { self.hint.update_line(""); Some(b"\x1b".to_vec()) }
                        Key::Named(NamedKey::Tab)          => Some(b"\t".to_vec()),
                        Key::Named(NamedKey::ArrowUp)      => { self.hint.update_line(""); Some(b"\x1b[A".to_vec()) }
                        Key::Named(NamedKey::ArrowDown)    => { self.hint.update_line(""); Some(b"\x1b[B".to_vec()) }
                        Key::Named(NamedKey::ArrowRight)   => Some(b"\x1b[C".to_vec()),
                        Key::Named(NamedKey::ArrowLeft)    => Some(b"\x1b[D".to_vec()),
                        Key::Named(NamedKey::Home)         => Some(b"\x1b[H".to_vec()),
                        Key::Named(NamedKey::End)          => Some(b"\x1b[F".to_vec()),
                        Key::Named(NamedKey::PageUp)       => Some(b"\x1b[5~".to_vec()),
                        Key::Named(NamedKey::PageDown)     => Some(b"\x1b[6~".to_vec()),
                        Key::Named(NamedKey::Insert)       => Some(b"\x1b[2~".to_vec()),
                        Key::Named(NamedKey::Delete)       => Some(b"\x1b[3~".to_vec()),
                        Key::Named(NamedKey::F1)           => Some(b"\x1bOP".to_vec()),
                        Key::Named(NamedKey::F2)           => Some(b"\x1bOQ".to_vec()),
                        Key::Named(NamedKey::F3)           => Some(b"\x1bOR".to_vec()),
                        Key::Named(NamedKey::F4)           => Some(b"\x1bOS".to_vec()),
                        Key::Named(NamedKey::F5)           => Some(b"\x1b[15~".to_vec()),
                        Key::Named(NamedKey::F6)           => Some(b"\x1b[17~".to_vec()),
                        Key::Named(NamedKey::F7)           => Some(b"\x1b[18~".to_vec()),
                        Key::Named(NamedKey::F8)           => Some(b"\x1b[19~".to_vec()),
                        Key::Named(NamedKey::F9)           => Some(b"\x1b[20~".to_vec()),
                        Key::Named(NamedKey::F10)          => Some(b"\x1b[21~".to_vec()),
                        Key::Named(NamedKey::F11)          => Some(b"\x1b[23~".to_vec()),
                        Key::Named(NamedKey::F12)          => Some(b"\x1b[24~".to_vec()),
                        _ => None,
                    };
                    if let Some(b) = bytes {
                        if let Some(pane) = &mut self.pane { pane.scroll_off = 0; }
                        if let Some(pty)  = &mut self.pty  { let _ = pty.write_bytes(&b); }
                    }
                    return;
                };

                // Ctrl path lands here.
                if let Some(b) = bytes {
                    self.hint.update_line(""); // Ctrl sequences reset the hint.
                    if let Some(pane) = &mut self.pane { pane.scroll_off = 0; }
                    if let Some(pty)  = &mut self.pty  { let _ = pty.write_bytes(&b); }
                }
            }

            WindowEvent::RedrawRequested => {
                // Drain PTY output into pane, collect side-channel events.
                if let (Some(pty), Some(pane)) = (&self.pty, &mut self.pane) {
                    while let Ok(bytes) = pty.rx.try_recv() {
                        for event in pane.process_with_events(&bytes) {
                            match event {
                                rusty_mux::pane::PaneEvent::Cwd(payload) => {
                                    self.hint.set_cwd_from_osc7(&payload);
                                }
                            }
                        }
                        if self.pending_render.is_some() {
                            self.capture_buf.extend_from_slice(&bytes);
                            self.capture_last_byte = Some(std::time::Instant::now());
                        }
                    }

                    // Finalise render after 200 ms of PTY silence (command done outputting).
                    if self.pending_render.is_some() {
                        let idle = self.capture_last_byte
                            .map(|t| t.elapsed().as_millis() >= 200)
                            .unwrap_or(false);
                        if idle {
                            let trigger = self.pending_render.take().unwrap();
                            let raw = String::from_utf8_lossy(&self.capture_buf).into_owned();
                            tracing::info!("render finalised: {:?}, {} bytes", trigger, raw.len());
                            let doc = build_render_doc(&trigger, &raw, pane.grid.width);
                            self.overlay = Some((doc, 0));
                            self.capture_buf.clear();
                            self.capture_last_byte = None;
                        }
                    }
                }
                if let (Some(gpu), Some(pane)) = (&mut self.gpu, &self.pane) {
                    let ghost = self.hint.hint()
                        .map(|h| h.ghost(&self.hint.line).to_owned())
                        .unwrap_or_default();
                    let overlay = self.overlay.as_ref().map(|(doc, scroll)| (doc, *scroll, self.overlay_sel));
                    gpu.render(pane, &self.font, self.font_px, self.cell_w, self.cell_h, self.baseline, self.top_inset, self.selection, &self.config, &ghost, self.popup.as_ref(), overlay);
                }
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(win) = &self.window {
            win.request_redraw();
        }
    }
}

// ── entry point ───────────────────────────────────────────────────────────────

pub struct TerminalWindow;

impl TerminalWindow {
    pub fn run(shell: &str, config: Config) {
        #[cfg(target_os = "macos")]
        let event_loop = {
            let mut builder = EventLoop::builder();
            builder.with_activation_policy(ActivationPolicy::Regular);
            builder.build().expect("event loop")
        };
        #[cfg(not(target_os = "macos"))]
        let event_loop = EventLoop::new().expect("event loop");

        event_loop.set_control_flow(ControlFlow::Poll);
        let mut app = App::new(shell, config);
        event_loop.run_app(&mut app).expect("event loop error");
    }
}

// ── software rasterizer ───────────────────────────────────────────────────────

/// Read a row from the grid as a trimmed string.
fn read_cursor_row(grid: &Grid, row: usize) -> String {
    if row >= grid.height { return String::new(); }
    let mut s: String = (0..grid.width)
        .map(|col| grid.get(col, row).ch)
        .collect();
    // Trim trailing spaces — the grid pads all cells with spaces.
    s.truncate(s.trim_end().len());
    s
}

/// Extract text from RenderDoc lines for the selected row range (screen-relative).
/// If no selection, returns all visible text.
fn overlay_selected_text(doc: &RenderDoc, scroll: usize, sel: Option<(usize, usize)>) -> String {
    let (lo, hi) = sel.unwrap_or((0, doc.lines.len().saturating_sub(1)));
    let doc_lo = (scroll + lo).min(doc.lines.len());
    let doc_hi = (scroll + hi + 1).min(doc.lines.len());
    doc.lines[doc_lo..doc_hi]
        .iter()
        .map(|spans| {
            let line: String = spans.iter().map(|s| s.text.as_str()).collect();
            line.trim_end().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn rasterize_glyph(ch: char, px: usize, py: usize, fg: [u8;4], bg: [u8;4], baseline: usize, font: &Font, font_px: f32, fb_w: usize, fb_h: usize, buf: &mut [u8]) {
    let (m, bitmap) = font.rasterize(ch, font_px);
    if m.width == 0 || m.height == 0 { return; }
    let gx = px as i32 + m.xmin;
    let gy = py as i32 + baseline as i32 - m.height as i32 - m.ymin;
    for by in 0..m.height {
        let y = gy + by as i32;
        if y < 0 || y as usize >= fb_h { continue; }
        let rb = y as usize * fb_w;
        for bx in 0..m.width {
            let a = bitmap[by * m.width + bx];
            if a == 0 { continue; }
            let x = gx + bx as i32;
            if x < 0 || x as usize >= fb_w { continue; }
            let i   = (rb + x as usize) * 4;
            let a32 = a as u32;
            let blend = |f: u8, b: u8| -> u8 { ((f as u32 * a32 + b as u32 * (255 - a32)) / 255) as u8 };
            buf[i]     = blend(fg[0], bg[0]);
            buf[i + 1] = blend(fg[1], bg[1]);
            buf[i + 2] = blend(fg[2], bg[2]);
            buf[i + 3] = 0xff;
        }
    }
}

fn render_text_row(text: &str, row: usize, cell_h: usize, cell_w: usize, baseline: usize, font: &Font, font_px: f32, fb_w: usize, fb_h: usize, buf: &mut [u8], fg: [u8;4], bg: [u8;4]) {
    let py = row * cell_h;
    let mut px = 0usize;
    for ch in text.chars() {
        if px + cell_w > fb_w { break; }
        if ch != ' ' {
            rasterize_glyph(ch, px, py, fg, bg, baseline, font, font_px, fb_w, fb_h, buf);
        }
        px += cell_w;
    }
}

fn build_render_doc(trigger: &RenderTrigger, raw: &str, width: usize) -> RenderDoc {
    let clean = strip_ansi(raw);
    match trigger {
        RenderTrigger::Markdown => rusty_render::markdown::render(strip_trailing_prompt(&clean), width),
        RenderTrigger::Json     => rusty_render::json::render(strip_trailing_prompt_json(&clean), width),
    }
}

/// Remove ANSI escape sequences from a string.
fn strip_ansi(s: &str) -> String {
    let mut out   = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.next() {
                Some('[') => { for c in chars.by_ref() { if c.is_ascii_alphabetic() { break; } } }
                Some(']') => { for c in chars.by_ref() { if c == '\x07' || c == '\x1b' { break; } } }
                _         => {}
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn copy_to_clipboard(text: &str) {
    use std::process::{Command, Stdio};
    use std::io::Write;
    if let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

fn paste_from_clipboard() -> Option<String> {
    let out = std::process::Command::new("pbpaste").output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn paint_framebuf(
    pane:      &Pane,
    font:      &Font,
    font_px:   f32,
    cell_w:    usize,
    cell_h:    usize,
    baseline:  usize,
    top_inset: usize,
    fb_w:      usize,
    fb_h:      usize,
    buf:       &mut [u8],
    selection: Option<Selection>,
    config:    &Config,
    ghost:     &str,
    popup:     Option<&PopupState>,
    overlay:   Option<(&RenderDoc, usize, Option<(usize,usize)>)>,
) {
    let palette   = &config.palette;
    let ansi16    = palette.to_ansi16();
    let cfg_bg    = palette.background.to_rgba();
    let cfg_fg    = palette.foreground.to_rgba();
    let cfg_cur   = palette.cursor.to_rgba();
    let cfg_selbg = palette.selection_bg.to_rgba();
    let cfg_selfg = palette.selection_fg.to_rgba();

    // ── Overlay rendering (replaces normal terminal view) ─────────────────────
    if let Some((doc, scroll_off, overlay_sel)) = overlay {
        let panel_bg: [u8; 4] = [0x1a, 0x1a, 0x2a, 0xff];
        buf.chunks_exact_mut(4).for_each(|p| p.copy_from_slice(&panel_bg));

        let visible_rows = fb_h / cell_h;
        let start_line   = scroll_off.min(doc.lines.len().saturating_sub(1));
        let end_line     = (start_line + visible_rows).min(doc.lines.len());

        // Status bar at the top.
        let status_bg: [u8; 4] = [0x26, 0x26, 0x40, 0xff];
        let status_fg: [u8; 4] = [0x88, 0x88, 0xaa, 0xff];
        for dy in 0..cell_h {
            for dx in 0..fb_w {
                let i = (dy * fb_w + dx) * 4;
                buf[i..i + 4].copy_from_slice(&status_bg);
            }
        }
        let sel_hint = if overlay_sel.is_some() { "  Cmd+C copy" } else { "" };
        let status_text = format!("  ↑↓ scroll  q/Esc dismiss   line {}/{}{}",
            start_line + 1, doc.lines.len(), sel_hint);
        render_text_row(&status_text, 0, cell_h, cell_w, baseline, font, font_px, fb_w, fb_h, buf, status_fg, status_bg);

        let sel_bg: [u8; 4] = [0x26, 0x4f, 0x78, 0xff];
        let sel_fg: [u8; 4] = [0xff, 0xff, 0xff, 0xff];

        // Content lines.
        for (i, line) in doc.lines[start_line..end_line].iter().enumerate() {
            let screen_row = i + 1; // row 0 is status bar
            let py = screen_row * cell_h;
            if py >= fb_h { break; }

            let is_selected = overlay_sel
                .map(|(lo, hi)| i >= lo && i <= hi)
                .unwrap_or(false);

            let mut px = cell_w; // 1-cell left margin
            for span in line {
                let fg = if is_selected { sel_fg } else { span.style.fg.map(|c| c.to_rgba()).unwrap_or(cfg_fg) };
                let bg = if is_selected { sel_bg } else { span.style.bg.map(|c| c.to_rgba()).unwrap_or(panel_bg) };

                // Fill span background.
                let span_w = span.text.chars().count() * cell_w;
                for dy in 0..cell_h {
                    let y = py + dy;
                    if y >= fb_h { break; }
                    for dx in 0..span_w {
                        let x = px + dx;
                        if x >= fb_w { break; }
                        let i = (y * fb_w + x) * 4;
                        buf[i..i + 4].copy_from_slice(&bg);
                    }
                }

                for ch in span.text.chars() {
                    if px + cell_w > fb_w { break; }
                    if ch != ' ' {
                        rasterize_glyph(ch, px, py, fg, bg, baseline, font, font_px, fb_w, fb_h, buf);
                    }
                    px += cell_w;
                }
            }
        }
        return; // don't draw normal terminal content
    }

    let resolve = |color: Color, is_fg: bool| -> [u8; 4] {
        color.resolve(is_fg, cfg_bg, cfg_fg, &ansi16)
    };

    buf.chunks_exact_mut(4).for_each(|p| p.copy_from_slice(&cfg_bg));

    let grid       = &pane.grid;
    let cursor     = pane.cursor;
    let scroll_off = pane.scroll_off;
    let total      = grid.total_rows();
    let view_start = total.saturating_sub(grid.height).saturating_sub(scroll_off);
    let scrolling  = scroll_off > 0;

    for screen_row in 0..grid.height {
        let src_row = view_start + screen_row;
        for col in 0..grid.width {
            let cell = *grid.scrollback_get(col, src_row);

            let is_cursor   = !scrolling && col == cursor.col && screen_row == cursor.row && cursor.visible;
            let is_selected = selection.map_or(false, |s| s.contains(col, screen_row));

            let (fg, bg): ([u8; 4], [u8; 4]) = if is_cursor {
                (cfg_bg, cfg_cur)
            } else if is_selected {
                (cfg_selfg, cfg_selbg)
            } else {
                (resolve(cell.fg, true), resolve(cell.bg, false))
            };

            let px = col * cell_w;
            let py = top_inset + screen_row * cell_h;

            for dy in 0..cell_h {
                let y = py + dy;
                if y >= fb_h { break; }
                for dx in 0..cell_w {
                    let x = px + dx;
                    if x >= fb_w { break; }
                    let i = (y * fb_w + x) * 4;
                    buf[i..i + 4].copy_from_slice(&bg);
                }
            }

            if cell.ch == ' ' { continue; }

            let (m, bitmap) = font.rasterize(cell.ch, font_px);
            if m.width == 0 || m.height == 0 { continue; }

            let glyph_top  = py as i32 + baseline as i32 - m.height as i32 - m.ymin;
            let glyph_left = px as i32 + m.xmin;

            for by in 0..m.height {
                let y = glyph_top + by as i32;
                if y < 0 || y as usize >= fb_h { continue; }
                let row_base = y as usize * fb_w;
                for bx in 0..m.width {
                    let a = bitmap[by * m.width + bx];
                    if a == 0 { continue; }
                    let x = glyph_left + bx as i32;
                    if x < 0 || x as usize >= fb_w { continue; }
                    let i = (row_base + x as usize) * 4;
                    let a32 = a as u32;
                    let blend = |f: u8, b: u8| -> u8 { ((f as u32 * a32 + b as u32 * (255 - a32)) / 255) as u8 };
                    buf[i]     = blend(fg[0], bg[0]);
                    buf[i + 1] = blend(fg[1], bg[1]);
                    buf[i + 2] = blend(fg[2], bg[2]);
                    buf[i + 3] = 0xff;
                }
            }
        }
    }

    // ── Ghost text (hint) ─────────────────────────────────────────────────────
    if !ghost.is_empty() && !pane.scroll_off > 0 {
        // Dim foreground — about 35% opacity over the background.
        let ghost_fg: [u8; 4] = [0x60, 0x60, 0x60, 0xff];
        let cursor = pane.cursor;
        let mut ghost_col = cursor.col + 1; // start one cell after cursor
        let ghost_row = cursor.row;
        let py = top_inset + ghost_row * cell_h;

        for ch in ghost.chars() {
            if ghost_col >= pane.grid.width { break; }
            if py >= fb_h { break; }

            // Clear cell background (use theme bg).
            let px = ghost_col * cell_w;
            for dy in 0..cell_h {
                let y = py + dy;
                if y >= fb_h { break; }
                for dx in 0..cell_w {
                    let x = px + dx;
                    if x >= fb_w { break; }
                    let i = (y * fb_w + x) * 4;
                    buf[i..i + 4].copy_from_slice(&cfg_bg);
                }
            }

            if ch != ' ' {
                let (m, bitmap) = font.rasterize(ch, font_px);
                let glyph_top  = py as i32 + baseline as i32 - m.height as i32 - m.ymin;
                let glyph_left = px as i32 + m.xmin;

                for by in 0..m.height {
                    let y = glyph_top + by as i32;
                    if y < 0 || y as usize >= fb_h { continue; }
                    let row_base = y as usize * fb_w;
                    for bx in 0..m.width {
                        let a = bitmap[by * m.width + bx];
                        if a == 0 { continue; }
                        let x = glyph_left + bx as i32;
                        if x < 0 || x as usize >= fb_w { continue; }
                        let i = (row_base + x as usize) * 4;
                        let a32 = a as u32;
                        let blend = |f: u8, b: u8| -> u8 { ((f as u32 * a32 + b as u32 * (255 - a32)) / 255) as u8 };
                        buf[i]     = blend(ghost_fg[0], cfg_bg[0]);
                        buf[i + 1] = blend(ghost_fg[1], cfg_bg[1]);
                        buf[i + 2] = blend(ghost_fg[2], cfg_bg[2]);
                        buf[i + 3] = 0xff;
                    }
                }
            }
            ghost_col += 1;
        }
    }

    // ── Completion popup ──────────────────────────────────────────────────────
    if let Some(popup) = popup {
        let cursor      = pane.cursor;
        let popup_x     = cursor.col * cell_w;
        let popup_y_top = top_inset + (cursor.row + 1) * cell_h; // just below cursor row

        // Popup dimensions in cells.
        let max_visible = 8usize;
        let visible_count = popup.entries.len().min(max_visible);
        let popup_w_cells = popup.entries.iter()
            .take(max_visible)
            .map(|e| e.label.len())
            .max()
            .unwrap_or(10)
            .max(10) + 2; // padding

        let popup_w_px = popup_w_cells * cell_w;
        let popup_h_px = visible_count * cell_h;

        // Clamp to screen.
        let px_start = popup_x.min(fb_w.saturating_sub(popup_w_px));

        // Colors.
        let row_bg:   [u8; 4] = [0x1e, 0x1e, 0x2e, 0xff]; // dark popup bg
        let row_fg:   [u8; 4] = [0xcc, 0xcc, 0xcc, 0xff];
        let sel_bg:   [u8; 4] = [0x26, 0x4f, 0x78, 0xff]; // selected row
        let sel_fg:   [u8; 4] = [0xff, 0xff, 0xff, 0xff];
        let dir_fg:   [u8; 4] = [0x8f, 0xc3, 0xff, 0xff]; // blue for dirs
        let cmd_fg:   [u8; 4] = [0xa8, 0xe0, 0x8a, 0xff]; // green for commands
        let hist_fg:  [u8; 4] = [0xe5, 0xc0, 0x76, 0xff]; // yellow for history

        let start_idx = if popup.selected >= max_visible {
            popup.selected - max_visible + 1
        } else {
            0
        };

        for (row_idx, entry) in popup.entries.iter().enumerate().skip(start_idx).take(max_visible) {
            let screen_row = row_idx - start_idx;
            let py = popup_y_top + screen_row * cell_h;
            if py + cell_h > fb_h { break; }

            let is_sel  = row_idx == popup.selected;
            let bg      = if is_sel { sel_bg } else { row_bg };
            let fg      = if is_sel {
                sel_fg
            } else {
                match entry.kind {
                    rusty_hint::EntryKind::Directory => dir_fg,
                    rusty_hint::EntryKind::Command   => cmd_fg,
                    rusty_hint::EntryKind::History   => hist_fg,
                    rusty_hint::EntryKind::File      => row_fg,
                }
            };

            // Fill row background.
            for dy in 0..cell_h {
                let y = py + dy;
                if y >= fb_h { break; }
                for dx in 0..popup_w_px {
                    let x = px_start + dx;
                    if x >= fb_w { break; }
                    let i = (y * fb_w + x) * 4;
                    buf[i..i + 4].copy_from_slice(&bg);
                }
            }

            // Render label text (with 1-cell left padding).
            let mut glyph_col = px_start + cell_w;
            for ch in entry.label.chars() {
                if glyph_col + cell_w > px_start + popup_w_px { break; }
                if ch == ' ' { glyph_col += cell_w; continue; }
                let (m, bitmap) = font.rasterize(ch, font_px);
                if m.width == 0 || m.height == 0 { glyph_col += cell_w; continue; }
                let gx = glyph_col as i32 + m.xmin;
                let gy = py as i32 + baseline as i32 - m.height as i32 - m.ymin;
                for by in 0..m.height {
                    let y = gy + by as i32;
                    if y < 0 || y as usize >= fb_h { continue; }
                    let rb = y as usize * fb_w;
                    for bx in 0..m.width {
                        let a = bitmap[by * m.width + bx];
                        if a == 0 { continue; }
                        let x = gx + bx as i32;
                        if x < 0 || x as usize >= fb_w { continue; }
                        let i   = (rb + x as usize) * 4;
                        let a32 = a as u32;
                        let blend = |f: u8, b: u8| -> u8 { ((f as u32 * a32 + b as u32 * (255 - a32)) / 255) as u8 };
                        buf[i]     = blend(fg[0], bg[0]);
                        buf[i + 1] = blend(fg[1], bg[1]);
                        buf[i + 2] = blend(fg[2], bg[2]);
                        buf[i + 3] = 0xff;
                    }
                }
                glyph_col += cell_w;
            }
        }
    }
}

// Fullscreen triangle. NDC Y-up, texture UV Y-down (origin = top-left).
//   vi=0: pos(-1, 1) uv(0,0)  top-left
//   vi=1: pos( 3, 1) uv(2,0)  far right (clipped, extends coverage)
//   vi=2: pos(-1,-3) uv(0,2)  far bottom (clipped, extends coverage)
// Interpolated UV at the four visible corners = exactly (0,0)..(1,1).
const BLIT_SHADER: &str = r#"
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

struct Vout {
    @builtin(position) pos: vec4<f32>,
    @location(0)       uv:  vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> Vout {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 3.0,  1.0),
        vec2<f32>(-1.0, -3.0),
    );
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(2.0, 0.0),
        vec2<f32>(0.0, 2.0),
    );
    var out: Vout;
    out.pos = vec4<f32>(positions[vi], 0.0, 1.0);
    out.uv  = uvs[vi];
    return out;
}

@fragment
fn fs_main(in: Vout) -> @location(0) vec4<f32> {
    return textureSample(t, s, in.uv);
}
"#;
