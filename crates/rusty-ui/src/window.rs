use std::collections::HashMap;
use std::sync::Arc;

use fontdue::{Font, FontSettings};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use rusty_config::Config;
use rusty_core::{Color, Grid};
use rusty_hint::HintEngine;
use rusty_mux::layout::{Rect, Split};
use rusty_mux::pane::Pane;
use rusty_mux::tab::FocusDir;
use rusty_mux::Session;
use rusty_render::{RenderDoc, RenderTrigger, detect_trigger, trigger::{strip_trailing_prompt, strip_trailing_prompt_json}};
use rusty_pty::{Pty, PtySize};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, KeyEvent, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{Window, WindowAttributes, WindowId},
};
#[cfg(target_os = "macos")]
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS, WindowAttributesExtMacOS};

const FONT_BYTES: &[u8] = include_bytes!("../assets/JetBrainsMono-Regular.ttf");

/// Height of the tab bar in cell units.
const TAB_BAR_ROWS: usize = 1;

// ── menu ──────────────────────────────────────────────────────────────────────

struct AppMenu {
    about:         MenuItem,
    check_updates: MenuItem,
    update_now:    MenuItem,
}

fn build_menu() -> AppMenu {
    let about         = MenuItem::new("About Rusty",           true, None);
    let check_updates = MenuItem::new("Check for Updates…",    true, None);
    let update_now    = MenuItem::new("Update Now",             false, None);

    let app_menu = Submenu::with_items(
        "Rusty",
        true,
        &[
            &about,
            &PredefinedMenuItem::separator(),
            &check_updates,
            &update_now,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ],
    ).expect("app submenu");

    let edit_menu = Submenu::with_items(
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
        ],
    ).expect("edit submenu");

    let window_menu = Submenu::with_items(
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
            &PredefinedMenuItem::fullscreen(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::bring_all_to_front(None),
        ],
    ).expect("window submenu");

    let menu = Menu::new();
    menu.append_items(&[&app_menu, &edit_menu, &window_menu])
        .expect("menu append");

    #[cfg(target_os = "macos")]
    menu.init_for_nsapp();

    AppMenu { about, check_updates, update_now }
}

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

// ── GPU state ─────────────────────────────────────────────────────────────────

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
    opacity:      f32,
}

impl Gpu {
    fn new(window: Arc<Window>, opacity: f32) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone()).expect("surface");

        let adapter = instance
            .enumerate_adapters(wgpu::Backends::METAL)
            .into_iter()
            .find(|a| a.is_surface_supported(&surface))
            .expect("no Metal adapter supports this surface");

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

        let caps        = surface.get_capabilities(&adapter);
        let surface_fmt = caps.formats[0];
        let alpha_mode = if opacity < 1.0 && caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PostMultiplied) {
            wgpu::CompositeAlphaMode::PostMultiplied
        } else {
            caps.alpha_modes[0]
        };
        let phys = window.inner_size();
        tracing::info!("surface fmt={surface_fmt:?} alpha={alpha_mode:?} phys={phys:?} opacity={opacity}");

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

        Self { device, queue, surface, surface_cfg, pipeline, bgl, sampler, screen_tex, bind_group, fb_w, fb_h, framebuf, opacity }
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

    fn render(&mut self, args: RenderArgs<'_>) {
        paint_framebuf(args, self.fb_w, self.fb_h, &mut self.framebuf, self.opacity);

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

// ── render args bundle ────────────────────────────────────────────────────────

struct RenderArgs<'a> {
    session:    &'a Session,
    font:       &'a Font,
    font_px:    f32,
    cell_w:     usize,
    cell_h:     usize,
    baseline:   usize,
    top_inset:  usize,
    left_inset: usize,
    selection:  Option<Selection>,
    config:     &'a Config,
    ghost:      &'a str,
    popup:      Option<&'a PopupState>,
    overlay:    Option<(&'a RenderDoc, usize, Option<(usize, usize)>)>,
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

// ── selection ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
struct Selection {
    start: (usize, usize),
    end:   (usize, usize),
}

impl Selection {
    fn normalised(&self) -> ((usize, usize), (usize, usize)) {
        let (sr, sc) = (self.start.1, self.start.0);
        let (er, ec) = (self.end.1,   self.end.0);
        if (sr, sc) <= (er, ec) { (self.start, self.end) } else { (self.end, self.start) }
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

#[cfg(target_os = "macos")]
const TITLEBAR_INSET_LOGICAL: f64 = 28.0;
#[cfg(not(target_os = "macos"))]
const TITLEBAR_INSET_LOGICAL: f64 = 0.0;

const LEFT_INSET_LOGICAL: f64 = 5.0;

// ── application ───────────────────────────────────────────────────────────────

struct App {
    shell:      String,
    config:     Config,
    font:       Font,
    gpu:        Option<Gpu>,
    window:     Option<Arc<Window>>,
    proxy:      EventLoopProxy<()>,
    menu:       Option<AppMenu>,
    /// One Pty per pane, keyed by pane ID.
    ptys:       HashMap<u32, Pty>,
    session:    Option<Session>,
    hint:       HintEngine,
    popup:      Option<PopupState>,
    pending_render:     Option<RenderTrigger>,
    capture_buf:        Vec<u8>,
    capture_last_byte:  Option<std::time::Instant>,
    overlay:            Option<(RenderDoc, usize)>,
    overlay_sel:        Option<(usize, usize)>,
    overlay_sel_start:  Option<usize>,
    font_px:    f32,
    cell_w:     usize,
    cell_h:     usize,
    baseline:   usize,
    top_inset:  usize,
    left_inset: usize,
    modifiers:  ModifiersState,
    selection:  Option<Selection>,
    selecting:  bool,
    cursor_pos: (f64, f64),
}

impl App {
    fn new(shell: &str, config: Config, proxy: EventLoopProxy<()>) -> Self {
        let font = Font::from_bytes(FONT_BYTES, FontSettings::default()).expect("font");
        let font_size = config.font.size;
        let fuzzy_history = config.hints.fuzzy_history;
        Self {
            shell:      shell.to_owned(),
            config,
            font,
            gpu:        None,
            window:     None,
            proxy,
            menu:       None,
            ptys:       HashMap::new(),
            session:    None,
            hint:       HintEngine::new(fuzzy_history),
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
            left_inset: 0,
            modifiers:  ModifiersState::default(),
            selection:  None,
            selecting:  false,
            cursor_pos: (0.0, 0.0),
        }
    }

    /// Usable cell dimensions (below title bar + tab bar).
    fn usable_cells(&self) -> (usize, usize) {
        let gpu = match &self.gpu { Some(g) => g, None => return (80, 24) };
        let cols = (gpu.fb_w.saturating_sub(self.left_inset)) / self.cell_w.max(1);
        let rows = (gpu.fb_h.saturating_sub(self.top_inset)) / self.cell_h.max(1);
        // subtract tab bar row
        let rows = rows.saturating_sub(TAB_BAR_ROWS);
        (cols.max(1), rows.max(1))
    }

    /// Pixel origin of the pane content area (below title bar + tab bar).
    fn pane_area_top(&self) -> usize {
        self.top_inset + TAB_BAR_ROWS * self.cell_h
    }

    /// Spawn a new PTY for a pane and register a watcher thread.
    fn spawn_pty(&mut self, pane_id: u32, cols: usize, rows: usize) {
        let pty = Pty::spawn(&self.shell, PtySize {
            cols: cols as u16, rows: rows as u16,
            px_w: (cols * self.cell_w) as u16,
            px_h: (rows * self.cell_h) as u16,
        }).expect("pty spawn");

        let watcher_notify = pty.notify.clone();
        let watcher_proxy  = self.proxy.clone();
        std::thread::Builder::new()
            .name(format!("pty-watcher-{pane_id}"))
            .spawn(move || {
                while watcher_notify.recv().is_ok() {
                    let _ = watcher_proxy.send_event(());
                }
            })
            .expect("pty-watcher thread");

        self.ptys.insert(pane_id, pty);
    }

    fn active_pane(&self) -> Option<&Pane> {
        self.session.as_ref()?.active_tab().active_pane()
    }

    fn active_pty_write(&mut self, bytes: &[u8]) {
        let id = self.session.as_ref().map(|s| s.active_tab().active_pane);
        if let Some(id) = id {
            if let Some(pty) = self.ptys.get_mut(&id) {
                let _ = pty.write_bytes(bytes);
            }
        }
    }

    fn pixel_to_cell(&self, x: f64, y: f64) -> (usize, usize) {
        let pane = match self.active_pane() { Some(p) => p, None => return (0, 0) };
        let col = ((x as usize).saturating_sub(self.left_inset) / self.cell_w).min(pane.grid.width.saturating_sub(1));
        let row = ((y as usize).saturating_sub(self.pane_area_top()) / self.cell_h).min(pane.grid.height.saturating_sub(1));
        (col, row)
    }

    fn send_mouse_report(&mut self, btn: u8, press: bool, x: f64, y: f64) {
        let reporting = self.active_pane().map_or(false, |p| p.mouse_report);
        if !reporting { return; }
        let (col, row) = self.pixel_to_cell(x, y);
        let seq = format!("\x1b[<{};{};{}{}", btn, col + 1, row + 1, if press { 'M' } else { 'm' });
        self.active_pty_write(seq.as_bytes());
    }

    fn selected_text(&self) -> Option<String> {
        let sel  = self.selection?;
        let pane = self.active_pane()?;
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
            out.push_str(line.trim_end_matches(' '));
            if row < er { out.push('\n'); }
        }
        Some(out)
    }

    /// Open a new tab: create it in the session, spawn a PTY, wake the window.
    fn open_new_tab(&mut self) {
        let (cols, rows) = self.usable_cells();
        let pane_id = match &mut self.session {
            Some(s) => {
                let tab_idx = s.new_tab();
                s.tabs[tab_idx].active_pane
            }
            None => return,
        };
        self.spawn_pty(pane_id, cols, rows);
        self.hint.update_line("");
        self.selection = None;
        self.overlay   = None;
    }

    /// Close the active tab. If the session is empty, exit.
    fn close_active_tab(&mut self, event_loop: &ActiveEventLoop) {
        if self.session.is_none() { return; }
        let session = self.session.as_mut().unwrap();

        // Collect all pane IDs in this tab before closing.
        let tab_pane_ids: Vec<u32> = session.active_tab().layout.pane_ids();
        let empty = session.close_active_tab();

        for id in tab_pane_ids {
            self.ptys.remove(&id);
        }

        if empty {
            self.session = None; // prevent re-entry on queued key events
            event_loop.exit();
        } else {
            self.hint.update_line("");
            self.selection = None;
            self.overlay   = None;
        }
    }

    /// Split the active pane horizontally (side by side) or vertically (stacked).
    fn split_pane(&mut self, direction: Split) {
        let (cols, rows) = self.usable_cells();
        let new_pane_id = match &mut self.session {
            Some(s) => {
                let new_id = s.alloc_pane_id();
                let id = s.active_tab_mut().split(direction, new_id, cols, rows);
                s.active_tab_mut().resize(cols, rows);
                id
            }
            None => return,
        };
        // spawn half-size PTY (will be resized by first Resized event anyway)
        let half_cols = cols / 2;
        let half_rows = rows / 2;
        self.spawn_pty(new_pane_id, half_cols.max(1), half_rows.max(1));
        self.hint.update_line("");
        self.selection = None;
        self.overlay   = None;
    }

    /// Close the active pane. If the tab is now empty, close the tab.
    fn close_active_pane(&mut self, event_loop: &ActiveEventLoop) {
        // Guard: if the session is gone (already exiting), do nothing.
        if self.session.is_none() { return; }

        let (removed_id, tab_empty) = {
            let session = self.session.as_mut().unwrap();
            let id      = session.active_tab().active_pane;
            let empty   = session.active_tab_mut().close_active_pane();
            (id, empty)
        };

        self.ptys.remove(&removed_id);

        if tab_empty {
            self.close_active_tab(event_loop);
        } else {
            self.hint.update_line("");
            self.selection = None;
            self.overlay   = None;
        }
    }

    /// Resize all panes and PTYs to fit current window size.
    fn resize_all(&mut self) {
        let (cols, rows) = self.usable_cells();
        if let Some(session) = &mut self.session {
            session.resize(cols, rows);
        }
        // Sync every PTY to its pane's new size.
        if let Some(session) = &self.session {
            for tab in &session.tabs {
                let rects = tab.layout.rects(0, 0, cols, rows);
                for (pane_id, rect) in rects {
                    if let Some(pty) = self.ptys.get(&pane_id) {
                        let _ = pty.resize(PtySize {
                            cols: rect.w as u16,
                            rows: rect.h as u16,
                            px_w: (rect.w * self.cell_w) as u16,
                            px_h: (rect.h * self.cell_h) as u16,
                        });
                    }
                }
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.menu.is_none() {
            self.menu = Some(build_menu());
            #[cfg(target_os = "macos")]
            set_app_icon();
        }

        let opacity = self.config.window.opacity.clamp(0.0, 1.0);
        let attrs = {
            let base = WindowAttributes::default()
                .with_title("rusty")
                .with_inner_size(winit::dpi::LogicalSize::new(1024u32, 768u32))
                .with_transparent(opacity < 1.0);
            #[cfg(target_os = "macos")]
            let base = base
                .with_titlebar_transparent(true)
                .with_fullsize_content_view(true)
                .with_title_hidden(true)
                .with_movable_by_window_background(false);
            base
        };
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));

        let scale    = window.scale_factor() as f32;
        self.font_px = self.config.font.size * scale;
        (self.cell_w, self.cell_h, self.baseline) = measure_cell(&self.font, self.font_px);
        self.top_inset  = (TITLEBAR_INSET_LOGICAL * window.scale_factor()).round() as usize;
        self.left_inset = (LEFT_INSET_LOGICAL     * window.scale_factor()).round() as usize;
        tracing::info!("scale={scale} font_px={} cell={}×{} baseline={} top_inset={} left_inset={}",
            self.font_px, self.cell_w, self.cell_h, self.baseline, self.top_inset, self.left_inset);

        let phys = window.inner_size();
        let cols = ((phys.width  as usize).saturating_sub(self.left_inset)) / self.cell_w.max(1);
        let rows = ((phys.height as usize).saturating_sub(self.top_inset))  / self.cell_h.max(1);
        let rows = rows.saturating_sub(TAB_BAR_ROWS).max(1);

        let session = Session::new(0, cols, rows);
        let pane_id = session.active_tab().active_pane;

        self.gpu     = Some(Gpu::new(window.clone(), opacity));
        self.session = Some(session);

        self.spawn_pty(pane_id, cols, rows);

        window.request_redraw();
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
                self.resize_all();
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y)   => y as f32,
                    winit::event::MouseScrollDelta::PixelDelta(pos)   => pos.y as f32 / self.cell_h as f32,
                };
                if let Some((doc, scroll)) = &mut self.overlay {
                    let max_scroll = doc.lines.len().saturating_sub(1);
                    if lines > 0.0 { *scroll = scroll.saturating_sub(lines.ceil() as usize); }
                    else           { *scroll = (*scroll + (-lines).ceil() as usize).min(max_scroll); }
                    return;
                }
                let reporting = self.active_pane().map_or(false, |p| p.mouse_report);
                if reporting {
                    let (cx, cy) = self.cursor_pos;
                    let n = lines.abs().ceil() as usize;
                    let btn = if lines > 0.0 { 64u8 } else { 65u8 };
                    for _ in 0..n { self.send_mouse_report(btn, true, cx, cy); }
                    return;
                }
                if self.active_pane().map_or(false, |p| p.grid.in_alt_screen) {
                    let n   = lines.abs().ceil() as usize;
                    let app = self.active_pane().map_or(false, |p| p.app_cursor);
                    let (up, dn) = if app { (b"\x1bOA".as_ref(), b"\x1bOB".as_ref()) } else { (b"\x1b[A".as_ref(), b"\x1b[B".as_ref()) };
                    let seq = if lines > 0.0 { up } else { dn };
                    for _ in 0..n { self.active_pty_write(seq); }
                    return;
                }
                if let Some(session) = &mut self.session {
                    let pane = session.active_tab_mut().active_pane_mut();
                    if let Some(p) = pane {
                        if lines > 0.0 { p.scroll_up_view(lines.ceil() as usize); }
                        else           { p.scroll_down_view((-lines).ceil() as usize); }
                    }
                }
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = (position.x, position.y);
                let fb_h = self.gpu.as_ref().map(|g| g.fb_h).unwrap_or(1);
                let scroll_zone = self.cell_h * 2;

                if self.selecting && self.active_pane().map_or(false, |p| p.mouse_report) {
                    self.send_mouse_report(32, true, position.x, position.y);
                    return;
                }

                if self.overlay.is_some() {
                    if self.overlay_sel_start.is_some() {
                        let screen_row = ((position.y as usize).saturating_sub(self.top_inset) / self.cell_h).saturating_sub(1);
                        if let Some(start) = self.overlay_sel_start {
                            let lo = start.min(screen_row);
                            let hi = start.max(screen_row);
                            self.overlay_sel = Some((lo, hi));
                        }
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
                    if let Some(sel) = &mut self.selection { sel.end = cell; }
                    if let Some(session) = &mut self.session {
                        let pane_area_top = self.top_inset + TAB_BAR_ROWS * self.cell_h;
                        if let Some(p) = session.active_tab_mut().active_pane_mut() {
                            if position.y as usize <= pane_area_top + scroll_zone {
                                p.scroll_up_view(1);
                            } else if position.y as usize >= fb_h.saturating_sub(scroll_zone) {
                                p.scroll_down_view(1);
                            }
                        }
                    }
                }
            }

            WindowEvent::MouseInput { state, button, .. } => {
                let btn_code = match button {
                    winit::event::MouseButton::Left   => 0u8,
                    winit::event::MouseButton::Middle => 1u8,
                    winit::event::MouseButton::Right  => 2u8,
                    _ => return,
                };
                let (cx, cy) = self.cursor_pos;
                let press = state == ElementState::Pressed;

                if self.overlay.is_some() && btn_code == 0 {
                    match state {
                        ElementState::Pressed => {
                            let screen_row = ((cy as usize).saturating_sub(self.top_inset) / self.cell_h).saturating_sub(1);
                            self.overlay_sel_start = Some(screen_row);
                            self.overlay_sel = Some((screen_row, screen_row));
                        }
                        ElementState::Released => { self.overlay_sel_start = None; }
                    }
                    return;
                }

                // Tab bar click — check before the drag zone so clicks register.
                let tab_bar_top = self.top_inset;
                let tab_bar_bot = self.top_inset + TAB_BAR_ROWS * self.cell_h;
                if btn_code == 0 && press && cy as usize >= tab_bar_top && (cy as usize) < tab_bar_bot {
                    if let Some(session) = &mut self.session {
                        let n_tabs = session.tabs.len();
                        let tab_w = self.gpu.as_ref().map(|g| g.fb_w / n_tabs.max(1)).unwrap_or(100);
                        let clicked = (cx as usize) / tab_w.max(1);
                        if clicked < n_tabs {
                            session.active_tab = clicked;
                            self.hint.update_line("");
                            self.selection = None;
                        }
                    }
                    return;
                }

                // Drag window via empty titlebar area (above tab bar).
                if btn_code == 0 && press && (cy as usize) < tab_bar_top {
                    if let Some(win) = &self.window { let _ = win.drag_window(); }
                    return;
                }

                let reporting = self.active_pane().map_or(false, |p| p.mouse_report);
                if reporting {
                    self.send_mouse_report(btn_code, press, cx, cy);
                    return;
                }

                if btn_code == 0 {
                    match state {
                        ElementState::Pressed => {
                            self.selecting = true;
                            self.selection = None;
                            let cell = self.pixel_to_cell(cx, cy);
                            self.selection = Some(Selection { start: cell, end: cell });
                        }
                        ElementState::Released => {
                            self.selecting = false;
                            if let Some(sel) = &self.selection {
                                if sel.start == sel.end { self.selection = None; }
                            }
                        }
                    }
                }
            }

            WindowEvent::KeyboardInput {
                event: KeyEvent { logical_key, text, state: ElementState::Pressed, .. }, ..
            } => {
                // Overlay navigation.
                if let Some((doc, scroll)) = &mut self.overlay {
                    let max_scroll = doc.lines.len().saturating_sub(1);
                    let cmd = self.modifiers.super_key();
                    match &logical_key {
                        Key::Named(NamedKey::Escape) | Key::Named(NamedKey::Enter) => {
                            self.overlay = None; self.overlay_sel = None; return;
                        }
                        Key::Character(s) if s.as_str() == "q" => {
                            self.overlay = None; self.overlay_sel = None; return;
                        }
                        Key::Character(s) if s.as_str() == "c" && cmd => {
                            let text = overlay_selected_text(doc, *scroll, self.overlay_sel);
                            copy_to_clipboard(&text); return;
                        }
                        Key::Named(NamedKey::ArrowUp)   => { *scroll = scroll.saturating_sub(1); return; }
                        Key::Named(NamedKey::ArrowDown) => { *scroll = (*scroll + 1).min(max_scroll); return; }
                        Key::Named(NamedKey::PageUp)    => { *scroll = scroll.saturating_sub(10); return; }
                        Key::Named(NamedKey::PageDown)  => { *scroll = (*scroll + 10).min(max_scroll); return; }
                        Key::Named(
                            NamedKey::Super | NamedKey::Shift | NamedKey::Control | NamedKey::Alt |
                            NamedKey::CapsLock | NamedKey::Meta
                        ) => { return; }
                        _ => { self.overlay = None; self.overlay_sel = None; }
                    }
                }

                let ctrl = self.modifiers.control_key();
                let cmd  = self.modifiers.super_key();
                let alt  = self.modifiers.alt_key();

                // ── Mux shortcuts (Cmd-based) ──────────────────────────────────
                if cmd {
                    if let Key::Character(s) = &logical_key {
                        match s.as_str() {
                            // Cmd+C — copy selection.
                            "c" => {
                                if let Some(text) = self.selected_text() { copy_to_clipboard(&text); }
                                return;
                            }
                            // Cmd+V — paste.
                            "v" => {
                                if let Some(text) = paste_from_clipboard() {
                                    if let Some(session) = &mut self.session {
                                        if let Some(p) = session.active_tab_mut().active_pane_mut() {
                                            p.scroll_off = 0;
                                        }
                                    }
                                    self.active_pty_write(text.as_bytes());
                                }
                                return;
                            }
                            // Cmd+T — new tab.
                            "t" => { self.open_new_tab(); return; }
                            // Cmd+W — close active pane; if last pane, close tab; if last tab, exit.
                            "w" => { self.close_active_pane(event_loop); return; }
                            // Cmd+D — split vertical (side by side).
                            // Cmd+Shift+D — split horizontal (stacked). OS delivers "D" when shift held.
                            "d" => { self.split_pane(Split::Horizontal); return; }
                            "D" => { self.split_pane(Split::Vertical);   return; }
                            // Cmd+] / Cmd+[ — next/prev tab.
                            "]" => { if let Some(s) = &mut self.session { s.next_tab(); } return; }
                            "[" => { if let Some(s) = &mut self.session { s.prev_tab(); } return; }
                            _ => {}
                        }
                    }
                    // Cmd+Option+Arrow — navigate panes.
                    if alt {
                        let (cols, rows) = self.usable_cells();
                        let dir = match &logical_key {
                            Key::Named(NamedKey::ArrowRight) => Some(FocusDir::Right),
                            Key::Named(NamedKey::ArrowLeft)  => Some(FocusDir::Left),
                            Key::Named(NamedKey::ArrowUp)    => Some(FocusDir::Up),
                            Key::Named(NamedKey::ArrowDown)  => Some(FocusDir::Down),
                            _ => None,
                        };
                        if let (Some(dir), Some(session)) = (dir, &mut self.session) {
                            session.active_tab_mut().focus_direction(dir, cols, rows);
                            self.hint.update_line("");
                            return;
                        }
                    }
                }

                let bytes: Option<Vec<u8>> = if ctrl {
                    match &logical_key {
                        Key::Character(s) => {
                            let ch = s.as_str().chars().next().unwrap_or('\0').to_ascii_uppercase();
                            if ('A'..='Z').contains(&ch) {
                                Some(vec![ch as u8 - b'A' + 1])
                            } else {
                                match ch {
                                    '[' => Some(b"\x1b".to_vec()),
                                    '\\' => Some(b"\x1c".to_vec()),
                                    ']' => Some(b"\x1d".to_vec()),
                                    '^' => Some(b"\x1e".to_vec()),
                                    '_' => Some(b"\x1f".to_vec()),
                                    ' ' => Some(b"\x00".to_vec()),
                                    _ => None,
                                }
                            }
                        }
                        _ => None,
                    }
                } else {
                    // Popup navigation.
                    if let Some(popup) = &mut self.popup {
                        match &logical_key {
                            Key::Named(NamedKey::ArrowUp)   => { popup.move_up();   return; }
                            Key::Named(NamedKey::ArrowDown) => { popup.move_down(); return; }
                            Key::Named(NamedKey::Escape)    => { self.popup = None; return; }
                            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Tab) => {
                                let insert = popup.selected_insert().to_owned();
                                self.popup = None;
                                // Replace only the last token, not the whole line.
                                // e.g. line="git -", insert="--version" → send "--version", replacing "-"
                                let current_line = &self.hint.line;
                                let last_token_start = current_line
                                    .rfind(|c: char| c == ' ')
                                    .map(|i| i + 1)
                                    .unwrap_or(0);
                                let prefix_to_keep = &current_line[..last_token_start];
                                let new_line = format!("{}{}", prefix_to_keep, insert);
                                // Send only the characters needed to complete: erase the last
                                // token and write the full insert.
                                let last_token_len = current_line.len() - last_token_start;
                                let erase = "\x08".repeat(last_token_len);
                                let write_str = format!("{}{}", erase, insert);
                                self.hint.line = new_line;
                                if let Some(session) = &mut self.session {
                                    if let Some(p) = session.active_tab_mut().active_pane_mut() { p.scroll_off = 0; }
                                }
                                self.active_pty_write(write_str.as_bytes());
                                return;
                            }
                            _ => { self.popup = None; }
                        }
                    }

                    // Tab — completions / ghost hint.
                    if matches!(&logical_key, Key::Named(NamedKey::Tab)) {
                        // Sync hint.line from the terminal grid before computing
                        // completions — corrects drift from Ctrl+C, Ctrl+U,
                        // shell history navigation, paste, etc.
                        // We read up to the cursor column (not trim_end) so that
                        // a trailing space ("git ") is preserved, which is what
                        // tells the completion engine we're in argument position.
                        if let Some((grid_line, cursor_col)) = self.active_pane()
                            .map(|p| (read_cursor_row_to_col(&p.grid, p.cursor.row, p.cursor.col), p.cursor.col))
                            .filter(|(s, _)| !s.trim().is_empty())
                        {
                            let _ = cursor_col;
                            let command_part = strip_prompt(&grid_line);
                            self.hint.update_line(command_part);
                        }
                        let entries = self.hint.completions();
                        if !entries.is_empty() { self.popup = PopupState::new(entries); return; }
                        if let Some(suffix) = self.hint.accept_ghost() {
                            if let Some(session) = &mut self.session {
                                if let Some(p) = session.active_tab_mut().active_pane_mut() { p.scroll_off = 0; }
                            }
                            self.active_pty_write(suffix.as_bytes());
                            return;
                        }
                    }

                    // ArrowRight — accept ghost.
                    if matches!(&logical_key, Key::Named(NamedKey::ArrowRight)) {
                        if let Some(suffix) = self.hint.accept_ghost() {
                            if let Some(session) = &mut self.session {
                                if let Some(p) = session.active_tab_mut().active_pane_mut() { p.scroll_off = 0; }
                            }
                            self.active_pty_write(suffix.as_bytes());
                            return;
                        }
                    }

                    if let Some(t) = &text {
                        let s = t.as_str();
                        match s {
                            "\r" | "\n" => {
                                let grid_line = self.active_pane()
                                    .map(|p| read_cursor_row(&p.grid, p.cursor.row))
                                    .unwrap_or_default();
                                let check_line = if grid_line.trim().is_empty() { &self.hint.line } else { &grid_line };
                                if let Some(trigger) = detect_trigger(check_line) {
                                    self.pending_render    = Some(trigger);
                                    self.capture_buf       = Vec::new();
                                    self.capture_last_byte = None;
                                    self.overlay           = None;
                                }
                                self.hint.commit();
                            }
                            "\x7f" => {
                                let mut line = self.hint.line.clone();
                                line.pop();
                                self.hint.update_line(&line);
                            }
                            "\x1b" => { self.hint.update_line(""); }
                            _ => {
                                let mut line = self.hint.line.clone();
                                line.push_str(s);
                                self.hint.update_line(&line);
                            }
                        }
                        self.popup = None;
                        if let Some(session) = &mut self.session {
                            if let Some(p) = session.active_tab_mut().active_pane_mut() { p.scroll_off = 0; }
                        }
                        self.active_pty_write(s.as_bytes());
                        return;
                    }

                    let app_cursor = self.active_pane().map_or(false, |p| p.app_cursor);
                    let bytes: Option<Vec<u8>> = match &logical_key {
                        Key::Named(NamedKey::Enter) => {
                            let grid_line = self.active_pane()
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
                        Key::Named(NamedKey::Backspace) => {
                            let mut line = self.hint.line.clone();
                            line.pop();
                            self.hint.update_line(&line);
                            Some(b"\x7f".to_vec())
                        }
                        Key::Named(NamedKey::Escape)    => { self.hint.update_line(""); Some(b"\x1b".to_vec()) }
                        Key::Named(NamedKey::Tab)        => Some(b"\t".to_vec()),
                        Key::Named(NamedKey::ArrowUp)    => { self.hint.update_line(""); Some(if app_cursor { b"\x1bOA".to_vec() } else { b"\x1b[A".to_vec() }) }
                        Key::Named(NamedKey::ArrowDown)  => { self.hint.update_line(""); Some(if app_cursor { b"\x1bOB".to_vec() } else { b"\x1b[B".to_vec() }) }
                        Key::Named(NamedKey::ArrowRight) => Some(if app_cursor { b"\x1bOC".to_vec() } else { b"\x1b[C".to_vec() }),
                        Key::Named(NamedKey::ArrowLeft)  => Some(if app_cursor { b"\x1bOD".to_vec() } else { b"\x1b[D".to_vec() }),
                        Key::Named(NamedKey::Home)       => Some(b"\x1b[H".to_vec()),
                        Key::Named(NamedKey::End)        => Some(b"\x1b[F".to_vec()),
                        Key::Named(NamedKey::PageUp)     => Some(b"\x1b[5~".to_vec()),
                        Key::Named(NamedKey::PageDown)   => Some(b"\x1b[6~".to_vec()),
                        Key::Named(NamedKey::Insert)     => Some(b"\x1b[2~".to_vec()),
                        Key::Named(NamedKey::Delete)     => Some(b"\x1b[3~".to_vec()),
                        Key::Named(NamedKey::F1)         => Some(b"\x1bOP".to_vec()),
                        Key::Named(NamedKey::F2)         => Some(b"\x1bOQ".to_vec()),
                        Key::Named(NamedKey::F3)         => Some(b"\x1bOR".to_vec()),
                        Key::Named(NamedKey::F4)         => Some(b"\x1bOS".to_vec()),
                        Key::Named(NamedKey::F5)         => Some(b"\x1b[15~".to_vec()),
                        Key::Named(NamedKey::F6)         => Some(b"\x1b[17~".to_vec()),
                        Key::Named(NamedKey::F7)         => Some(b"\x1b[18~".to_vec()),
                        Key::Named(NamedKey::F8)         => Some(b"\x1b[19~".to_vec()),
                        Key::Named(NamedKey::F9)         => Some(b"\x1b[20~".to_vec()),
                        Key::Named(NamedKey::F10)        => Some(b"\x1b[21~".to_vec()),
                        Key::Named(NamedKey::F11)        => Some(b"\x1b[23~".to_vec()),
                        Key::Named(NamedKey::F12)        => Some(b"\x1b[24~".to_vec()),
                        _ => None,
                    };
                    if let Some(b) = bytes {
                        if let Some(session) = &mut self.session {
                            if let Some(p) = session.active_tab_mut().active_pane_mut() { p.scroll_off = 0; }
                        }
                        self.active_pty_write(&b);
                    }
                    return;
                };

                // Ctrl path.
                if let Some(b) = bytes {
                    self.hint.update_line("");
                    if let Some(session) = &mut self.session {
                        if let Some(p) = session.active_tab_mut().active_pane_mut() { p.scroll_off = 0; }
                    }
                    self.active_pty_write(&b);
                }
            }

            WindowEvent::RedrawRequested => {
                if let (Some(gpu), Some(session)) = (&mut self.gpu, &self.session) {
                    let ghost = self.hint.hint()
                        .map(|h| h.ghost(&self.hint.line).to_owned())
                        .unwrap_or_default();
                    let overlay = self.overlay.as_ref().map(|(doc, scroll)| (doc, *scroll, self.overlay_sel));
                    gpu.render(RenderArgs {
                        session,
                        font:       &self.font,
                        font_px:    self.font_px,
                        cell_w:     self.cell_w,
                        cell_h:     self.cell_h,
                        baseline:   self.baseline,
                        top_inset:  self.top_inset,
                        left_inset: self.left_inset,
                        selection:  self.selection,
                        config:     &self.config,
                        ghost:      &ghost,
                        popup:      self.popup.as_ref(),
                        overlay,
                    });
                }
                return;
            }

            _ => {}
        }

        if let Some(win) = &self.window { win.request_redraw(); }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {}

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let mut dirty = false;

        if let Some(session) = &mut self.session {
            let mut pty_responses: Vec<(u32, Vec<u8>)> = Vec::new();
            // Collect (tab_index, cwd_payload) to apply after the inner loop releases tab borrows.
            let mut cwd_updates: Vec<(usize, String)> = Vec::new();

            for (tab_idx, tab) in session.tabs.iter_mut().enumerate() {
                for (pane_id, pane) in &mut tab.panes {
                    let pty = match self.ptys.get(pane_id) { Some(p) => p, None => continue };
                    while let Ok(bytes) = pty.rx.try_recv() {
                        let events = pane.process_with_events(&bytes);
                        for event in events {
                            match event {
                                rusty_mux::pane::PaneEvent::Cwd(payload) => {
                                    self.hint.set_cwd_from_osc7(&payload);
                                    cwd_updates.push((tab_idx, payload));
                                }
                                rusty_mux::pane::PaneEvent::PtyWrite(response) => {
                                    pty_responses.push((*pane_id, response));
                                }
                            }
                        }
                        if self.pending_render.is_some() && tab.active_pane == *pane_id {
                            self.capture_buf.extend_from_slice(&bytes);
                            self.capture_last_byte = Some(std::time::Instant::now());
                        }
                        pane.scroll_off = 0;
                        dirty = true;
                    }
                }
            }

            for (tab_idx, payload) in cwd_updates {
                if let Some(tab) = session.tabs.get_mut(tab_idx) {
                    tab.update_title_from_cwd(&payload);
                }
            }

            for (pane_id, response) in pty_responses {
                if let Some(pty) = self.ptys.get_mut(&pane_id) {
                    let _ = pty.write_bytes(&response);
                }
            }
        }

        // Finalise render after 200 ms of PTY silence.
        if self.pending_render.is_some() {
            let idle = self.capture_last_byte
                .map(|t| t.elapsed().as_millis() >= 200)
                .unwrap_or(false);
            if idle {
                let trigger = self.pending_render.take().unwrap();
                let raw = String::from_utf8_lossy(&self.capture_buf).into_owned();
                let width = self.active_pane().map_or(80, |p| p.grid.width);
                tracing::info!("render finalised: {:?}, {} bytes", trigger, raw.len());
                let doc = build_render_doc(&trigger, &raw, width);
                self.overlay = Some((doc, 0));
                self.capture_buf.clear();
                self.capture_last_byte = None;
                dirty = true;
            } else if self.capture_last_byte.is_some() {
                let elapsed   = self.capture_last_byte.unwrap().elapsed().as_millis() as u64;
                let remaining = 200u64.saturating_sub(elapsed);
                event_loop.set_control_flow(ControlFlow::WaitUntil(
                    std::time::Instant::now() + std::time::Duration::from_millis(remaining),
                ));
                return;
            }
        }

        if let Some(menu) = &self.menu {
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                let id = &event.id;
                if id == menu.about.id() {
                    #[cfg(target_os = "macos")]
                    show_about_panel();
                } else if id == menu.check_updates.id() {
                    std::thread::spawn(|| {
                        match crate::update::check() {
                            Ok(Some(tag)) => show_alert("Update Available",
                                &format!("Version {tag} is available.\nUse Rusty → Update Now to install, then restart.")),
                            Ok(None) => show_alert("Up to Date",
                                &format!("You're running the latest version ({}).", crate::update::CURRENT)),
                            Err(e) => show_alert("Update Check Failed", &format!("{e}")),
                        }
                    });
                } else if id == menu.update_now.id() {
                    std::thread::spawn(|| {
                        match crate::update::install() {
                            Ok(crate::update::UpdateStatus::Updated { to, .. }) => show_alert(
                                "Update Installed",
                                &format!("Updated to {to}. Restart Rusty to use the new version.")),
                            Ok(crate::update::UpdateStatus::AlreadyLatest) => show_alert(
                                "Up to Date",
                                &format!("You're already on the latest version ({}).", crate::update::CURRENT)),
                            Err(e) => show_alert("Update Failed", &format!("{e}")),
                        }
                    });
                }
            }
        }

        if dirty {
            if let Some(win) = &self.window { win.request_redraw(); }
        }
        event_loop.set_control_flow(ControlFlow::Wait);
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

        let proxy = event_loop.create_proxy();
        let mut app = App::new(shell, config, proxy);
        event_loop.run_app(&mut app).expect("event loop error");
    }
}

// ── native alert / about / icon ───────────────────────────────────────────────

fn show_alert(title: &str, message: &str) {
    let title   = title.replace('\'', "'\\''");
    let message = message.replace('\'', "'\\''");
    let script  = format!("display dialog \"{message}\" with title \"{title}\" buttons {{\"OK\"}} default button \"OK\"");
    let _ = std::process::Command::new("osascript").arg("-e").arg(&script).status();
}

#[cfg(target_os = "macos")]
fn show_about_panel() {
    use objc2::rc::autoreleasepool;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{MainThreadMarker, NSDictionary, NSString};
    autoreleasepool(|_pool| {
        let mtm = unsafe { MainThreadMarker::new_unchecked() };
        let app = NSApplication::sharedApplication(mtm);
        let name_key = NSString::from_str("ApplicationName");
        let name_val = NSString::from_str("Rusty");
        let ver_key  = NSString::from_str("ApplicationVersion");
        let ver_val  = NSString::from_str(crate::update::CURRENT);
        let keys: &[&NSString]  = &[&*name_key, &*ver_key];
        let vals: &[&AnyObject] = &[name_val.as_ref(), ver_val.as_ref()];
        let dict = NSDictionary::from_slices(keys, vals);
        unsafe { app.orderFrontStandardAboutPanelWithOptions(&*dict) };
    });
}

#[cfg(target_os = "macos")]
fn set_app_icon() {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::{MainThreadMarker, NSData};
    const ICON_BYTES: &[u8] = include_bytes!("../assets/icon.icns");
    autoreleasepool(|_pool| {
        let mtm = unsafe { MainThreadMarker::new_unchecked() };
        let data = unsafe {
            NSData::initWithBytes_length(
                mtm.alloc::<NSData>(),
                ICON_BYTES.as_ptr() as *const std::ffi::c_void,
                ICON_BYTES.len(),
            )
        };
        if let Some(image) = NSImage::initWithData(mtm.alloc::<NSImage>(), &data) {
            let app = NSApplication::sharedApplication(mtm);
            unsafe { app.setApplicationIconImage(Some(&image)) };
        }
    });
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn read_cursor_row(grid: &Grid, row: usize) -> String {
    if row >= grid.height { return String::new(); }
    let mut s: String = (0..grid.width).map(|col| grid.get(col, row).ch).collect();
    s.truncate(s.trim_end().len());
    s
}

/// Read the grid row up to `cursor_col` (exclusive), preserving trailing spaces.
/// This lets the completion engine distinguish "git" (command) from "git " (argument position).
fn read_cursor_row_to_col(grid: &Grid, row: usize, cursor_col: usize) -> String {
    if row >= grid.height { return String::new(); }
    let end = cursor_col.min(grid.width);
    (0..end).map(|col| grid.get(col, row).ch).collect()
}

/// Strip the shell prompt prefix from a grid row, returning only the command part.
/// Looks for the last occurrence of common prompt-terminating characters.
fn strip_prompt(line: &str) -> &str {
    // Find the last prompt terminator: '$', '#', '%', '❯', '➜', '>'
    let terminators = ['$', '#', '%', '❯', '➜', '>'];
    if let Some(pos) = line.rfind(|c| terminators.contains(&c)) {
        let after = &line[pos + line[pos..].chars().next().map_or(1, |c| c.len_utf8())..];
        after.trim_start()
    } else {
        line.trim()
    }
}

fn overlay_selected_text(doc: &RenderDoc, scroll: usize, sel: Option<(usize, usize)>) -> String {
    let (lo, hi) = sel.unwrap_or((0, doc.lines.len().saturating_sub(1)));
    let doc_lo = (scroll + lo).min(doc.lines.len());
    let doc_hi = (scroll + hi + 1).min(doc.lines.len());
    doc.lines[doc_lo..doc_hi]
        .iter()
        .map(|spans| { let line: String = spans.iter().map(|s| s.text.as_str()).collect(); line.trim_end().to_owned() })
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

fn strip_ansi(s: &str) -> String {
    let mut out   = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.next() {
                Some('[') => { for c in chars.by_ref() { if c.is_ascii_alphabetic() { break; } } }
                Some(']') => { for c in chars.by_ref() { if c == '\x07' || c == '\x1b' { break; } } }
                _ => {}
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
        if let Some(stdin) = child.stdin.as_mut() { let _ = stdin.write_all(text.as_bytes()); }
        let _ = child.wait();
    }
}

fn paste_from_clipboard() -> Option<String> {
    let out = std::process::Command::new("pbpaste").output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

// ── software rasterizer ───────────────────────────────────────────────────────

fn paint_framebuf(args: RenderArgs<'_>, fb_w: usize, fb_h: usize, buf: &mut [u8], opacity: f32) {
    let RenderArgs { session, font, font_px, cell_w, cell_h, baseline,
                     top_inset, left_inset, selection, config, ghost, popup, overlay } = args;

    let palette   = &config.palette;
    let ansi16    = palette.to_ansi16();
    let cfg_bg    = palette.background.to_rgba();
    let cfg_fg    = palette.foreground.to_rgba();
    let cfg_cur   = palette.cursor.to_rgba();
    let cfg_selbg = palette.selection_bg.to_rgba();
    let cfg_selfg = palette.selection_fg.to_rgba();

    // ── Overlay (replaces normal terminal view) ───────────────────────────────
    if let Some((doc, scroll_off, overlay_sel)) = overlay {
        let panel_bg: [u8; 4] = [0x1a, 0x1a, 0x2a, 0xff];
        buf.chunks_exact_mut(4).for_each(|p| p.copy_from_slice(&panel_bg));

        let visible_rows = fb_h / cell_h;
        let start_line   = scroll_off.min(doc.lines.len().saturating_sub(1));
        let end_line     = (start_line + visible_rows).min(doc.lines.len());

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

        for (i, line) in doc.lines[start_line..end_line].iter().enumerate() {
            let screen_row = i + 1;
            let py = screen_row * cell_h;
            if py >= fb_h { break; }
            let is_selected = overlay_sel.map(|(lo, hi)| i >= lo && i <= hi).unwrap_or(false);
            let mut px = cell_w;
            for span in line {
                let fg = if is_selected { sel_fg } else { span.style.fg.map(|c| c.to_rgba()).unwrap_or(cfg_fg) };
                let bg = if is_selected { sel_bg } else { span.style.bg.map(|c| c.to_rgba()).unwrap_or(panel_bg) };
                let span_w = span.text.chars().count() * cell_w;
                for dy in 0..cell_h {
                    let y = py + dy; if y >= fb_h { break; }
                    for dx in 0..span_w {
                        let x = px + dx; if x >= fb_w { break; }
                        let i = (y * fb_w + x) * 4;
                        buf[i..i + 4].copy_from_slice(&bg);
                    }
                }
                for ch in span.text.chars() {
                    if px + cell_w > fb_w { break; }
                    if ch != ' ' { rasterize_glyph(ch, px, py, fg, bg, baseline, font, font_px, fb_w, fb_h, buf); }
                    px += cell_w;
                }
            }
        }
        return;
    }

    // ── Background ───────────────────────────────────────────────────────────
    buf.chunks_exact_mut(4).for_each(|p| p.copy_from_slice(&cfg_bg));

    let tab        = session.active_tab();
    let active_id  = tab.active_pane;
    if tab.panes.is_empty() { return; }
    // Use the session's total stored dimensions — not a single pane's size,
    // which is only a fraction of the available area after splitting.
    let total_cols = session.cols();
    let total_rows = session.rows();
    let rects = tab.layout.rects(0, 0, total_cols, total_rows);

    let pane_area_top = top_inset + TAB_BAR_ROWS * cell_h;

    // ── Tab bar ───────────────────────────────────────────────────────────────
    {
        let tab_bar_bg:    [u8; 4] = config.tabs.bar_bg.to_rgba();
        let tab_active_bg: [u8; 4] = config.tabs.active_bg.to_rgba();
        let tab_fg:        [u8; 4] = config.tabs.bar_fg.to_rgba();
        let tab_active_fg: [u8; 4] = config.tabs.active_fg.to_rgba();

        // Fill tab bar background.
        for dy in 0..cell_h {
            let y = top_inset + dy;
            if y >= fb_h { break; }
            for dx in 0..fb_w {
                let i = (y * fb_w + dx) * 4;
                buf[i..i + 4].copy_from_slice(&tab_bar_bg);
            }
        }

        let n_tabs   = session.tabs.len();
        let tab_w_px = (fb_w / n_tabs.max(1)).max(cell_w * 6);

        for (ti, t) in session.tabs.iter().enumerate() {
            let is_active = ti == session.active_tab;
            let bg = if is_active { tab_active_bg } else { tab_bar_bg };
            let fg = if is_active { tab_active_fg } else { tab_fg };
            let tx = ti * tab_w_px;

            // Fill tab background.
            for dy in 0..cell_h {
                let y = top_inset + dy;
                if y >= fb_h { break; }
                for dx in 0..tab_w_px {
                    let x = tx + dx;
                    if x >= fb_w { break; }
                    let i = (y * fb_w + x) * 4;
                    buf[i..i + 4].copy_from_slice(&bg);
                }
            }

            // Tab label: use title if set, else "Tab N".
            let label = if t.title.is_empty() {
                format!(" Tab {} ", ti + 1)
            } else {
                format!(" {} ", t.title)
            };
            let py = top_inset;
            let mut px = tx + cell_w / 2;
            for ch in label.chars() {
                if px + cell_w > tx + tab_w_px { break; }
                if ch != ' ' {
                    rasterize_glyph(ch, px, py, fg, bg, baseline, font, font_px, fb_w, fb_h, buf);
                }
                px += cell_w;
            }

            // Separator between tabs.
            if ti + 1 < n_tabs {
                let sep_x = tx + tab_w_px - 1;
                let sep_col: [u8; 4] = config.tabs.separator.to_rgba();
                for dy in 0..cell_h {
                    let y = top_inset + dy;
                    if y >= fb_h || sep_x >= fb_w { break; }
                    let i = (y * fb_w + sep_x) * 4;
                    buf[i..i + 4].copy_from_slice(&sep_col);
                }
            }
        }
    }

    // ── Pane content ──────────────────────────────────────────────────────────
    let resolve = |color: Color, is_fg: bool| -> [u8; 4] {
        color.resolve(is_fg, cfg_bg, cfg_fg, &ansi16)
    };

    for (pane_id, rect) in &rects {
        let pane = match tab.panes.get(pane_id) { Some(p) => p, None => continue };
        let is_active_pane = *pane_id == active_id;

        let pane_px_x = left_inset + rect.x * cell_w;
        let pane_px_y = pane_area_top + rect.y * cell_h;

        let grid       = &pane.grid;
        let cursor     = pane.cursor;
        let scroll_off = pane.scroll_off;
        let total_r    = grid.total_rows();
        let view_start = total_r.saturating_sub(grid.height).saturating_sub(scroll_off);
        let scrolling  = scroll_off > 0;

        for screen_row in 0..grid.height {
            let src_row = view_start + screen_row;
            for col in 0..grid.width {
                let cell = *grid.scrollback_get(col, src_row);

                let is_cursor   = is_active_pane && !scrolling && col == cursor.col && screen_row == cursor.row && cursor.visible;
                let is_selected = is_active_pane && selection.map_or(false, |s| s.contains(col, screen_row));

                let (fg, bg): ([u8; 4], [u8; 4]) = if is_cursor {
                    (cfg_bg, cfg_cur)
                } else if is_selected {
                    (cfg_selfg, cfg_selbg)
                } else {
                    (resolve(cell.fg, true), resolve(cell.bg, false))
                };

                let px = pane_px_x + col * cell_w;
                let py = pane_px_y + screen_row * cell_h;

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

        // Active pane border highlight (1px top edge in accent colour).
        if is_active_pane && rects.len() > 1 {
            let accent: [u8; 4] = config.tabs.active_border.to_rgba();
            let y = pane_px_y;
            if y < fb_h {
                for dx in 0..(rect.w * cell_w) {
                    let x = pane_px_x + dx;
                    if x >= fb_w { break; }
                    let i = (y * fb_w + x) * 4;
                    buf[i..i + 4].copy_from_slice(&accent);
                }
            }
        }
    }

    // ── Pane dividers ─────────────────────────────────────────────────────────
    // Dividers live in the 1-cell gap between panes (set up by layout::rects).
    // We draw them by scanning for gaps between rect extents.
    paint_dividers(session, &rects, left_inset, pane_area_top, cell_w, cell_h, fb_w, fb_h, buf, config.tabs.separator.to_rgba());

    // ── Ghost text ────────────────────────────────────────────────────────────
    if let Some(pane) = tab.panes.get(&active_id) {
        if !ghost.is_empty() && pane.scroll_off == 0 {
            let ghost_fg: [u8; 4] = [0x60, 0x60, 0x60, 0xff];
            let cursor    = pane.cursor;
            let rect      = rects.iter().find(|(id, _)| *id == active_id).map(|(_, r)| *r);
            if let Some(rect) = rect {
                let pane_px_x = left_inset + rect.x * cell_w;
                let pane_px_y = pane_area_top + rect.y * cell_h;
                let mut ghost_col = cursor.col + 1;
                let ghost_row = cursor.row;
                let py = pane_px_y + ghost_row * cell_h;

                for ch in ghost.chars() {
                    if ghost_col >= pane.grid.width { break; }
                    if py >= fb_h { break; }
                    let px = pane_px_x + ghost_col * cell_w;
                    for dy in 0..cell_h {
                        let y = py + dy; if y >= fb_h { break; }
                        for dx in 0..cell_w {
                            let x = px + dx; if x >= fb_w { break; }
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
        }
    }

    // ── Completion popup ──────────────────────────────────────────────────────
    if let Some(popup) = popup {
        if let Some(pane) = tab.panes.get(&active_id) {
            let rect = rects.iter().find(|(id, _)| *id == active_id).map(|(_, r)| *r);
            if let Some(rect) = rect {
                let pane_px_x = left_inset + rect.x * cell_w;
                let pane_px_y = pane_area_top + rect.y * cell_h;
                let cursor    = pane.cursor;
                let popup_x   = pane_px_x + cursor.col * cell_w;
                let popup_y   = pane_px_y + (cursor.row + 1) * cell_h;

                let max_visible   = 8usize;
                let visible_count = popup.entries.len().min(max_visible);
                let popup_w_cells = popup.entries.iter().take(max_visible)
                    .map(|e| e.label.len()).max().unwrap_or(10).max(10) + 2;
                let popup_w_px = popup_w_cells * cell_w;
                let popup_h_px = visible_count * cell_h;
                let _ = popup_h_px;
                let px_start   = popup_x.min(fb_w.saturating_sub(popup_w_px));

                let row_bg:  [u8; 4] = [0x1e, 0x1e, 0x2e, 0xff];
                let row_fg:  [u8; 4] = [0xcc, 0xcc, 0xcc, 0xff];
                let sel_bg:  [u8; 4] = [0x26, 0x4f, 0x78, 0xff];
                let sel_fg:  [u8; 4] = [0xff, 0xff, 0xff, 0xff];
                let dir_fg:  [u8; 4] = [0x8f, 0xc3, 0xff, 0xff];
                let cmd_fg:  [u8; 4] = [0xa8, 0xe0, 0x8a, 0xff];
                let hist_fg: [u8; 4] = [0xe5, 0xc0, 0x76, 0xff];

                let start_idx = if popup.selected >= max_visible { popup.selected - max_visible + 1 } else { 0 };

                for (row_idx, entry) in popup.entries.iter().enumerate().skip(start_idx).take(max_visible) {
                    let screen_row = row_idx - start_idx;
                    let py = popup_y + screen_row * cell_h;
                    if py + cell_h > fb_h { break; }
                    let is_sel = row_idx == popup.selected;
                    let bg = if is_sel { sel_bg } else { row_bg };
                    let fg = if is_sel { sel_fg } else {
                        match entry.kind {
                            rusty_hint::EntryKind::Directory  => dir_fg,
                            rusty_hint::EntryKind::Command    => cmd_fg,
                            rusty_hint::EntryKind::History    => hist_fg,
                            rusty_hint::EntryKind::File       => row_fg,
                            rusty_hint::EntryKind::Flag       => cmd_fg,
                            rusty_hint::EntryKind::Subcommand => dir_fg,
                        }
                    };
                    for dy in 0..cell_h {
                        let y = py + dy; if y >= fb_h { break; }
                        for dx in 0..popup_w_px {
                            let x = px_start + dx; if x >= fb_w { break; }
                            let i = (y * fb_w + x) * 4;
                            buf[i..i + 4].copy_from_slice(&bg);
                        }
                    }
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
                                let i = (rb + x as usize) * 4;
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
    }

    // Opacity.
    if opacity < 1.0 {
        let a = (opacity * 255.0).round() as u8;
        for pixel in buf.chunks_exact_mut(4) { pixel[3] = a; }
    }
}

/// Draw 1-pixel dividers in the gaps between panes.
fn paint_dividers(
    session:       &Session,
    rects:         &[(u32, Rect)],
    left_inset:    usize,
    pane_area_top: usize,
    cell_w:        usize,
    cell_h:        usize,
    fb_w:          usize,
    fb_h:          usize,
    buf:           &mut [u8],
    separator:     [u8; 4],
) {
    if rects.len() < 2 { return; }
    let div_col = separator;
    let tab = session.active_tab();

    // For each pane, check if there's a gap to the right (horizontal split divider)
    // or below (vertical split divider) and paint it.
    for (id_a, rect_a) in rects {
        let right_edge_cell = rect_a.x + rect_a.w;
        let bot_edge_cell   = rect_a.y + rect_a.h;

        // Is there another pane that starts exactly one cell to the right?
        let has_right_neighbor = rects.iter().any(|(id_b, rect_b)| {
            id_b != id_a && rect_b.x == right_edge_cell + 1
                && rect_b.y < rect_a.y + rect_a.h
                && rect_b.y + rect_b.h > rect_a.y
        });
        if has_right_neighbor {
            let div_x = left_inset + right_edge_cell * cell_w;
            for row in 0..rect_a.h {
                for dy in 0..cell_h {
                    let y = pane_area_top + (rect_a.y + row) * cell_h + dy;
                    if y >= fb_h || div_x >= fb_w { continue; }
                    let i = (y * fb_w + div_x) * 4;
                    buf[i..i + 4].copy_from_slice(&div_col);
                }
            }
        }

        // Is there another pane that starts exactly one cell below?
        let has_bot_neighbor = rects.iter().any(|(id_b, rect_b)| {
            id_b != id_a && rect_b.y == bot_edge_cell + 1
                && rect_b.x < rect_a.x + rect_a.w
                && rect_b.x + rect_b.w > rect_a.x
        });
        if has_bot_neighbor {
            let div_y_start = pane_area_top + bot_edge_cell * cell_h;
            for dx in 0..(rect_a.w * cell_w) {
                let x = left_inset + rect_a.x * cell_w + dx;
                if x >= fb_w { break; }
                for dy in 0..cell_h {
                    let y = div_y_start + dy;
                    if y >= fb_h { continue; }
                    let i = (y * fb_w + x) * 4;
                    buf[i..i + 4].copy_from_slice(&div_col);
                }
            }
        }
    }
    let _ = tab;
}

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
