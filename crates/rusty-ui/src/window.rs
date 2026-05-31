use std::sync::Arc;

use fontdue::{Font, FontSettings};
use rusty_core::Color;
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
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};

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
        format:          wgpu::TextureFormat::Rgba8Unorm,
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
    fb_w:         usize,
    fb_h:         usize,
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

    fn render(&mut self, pane: &Pane, font: &Font, font_px: f32, cell_w: usize, cell_h: usize, baseline: usize) {
        paint_framebuf(pane, font, font_px, cell_w, cell_h, baseline, self.fb_w, self.fb_h, &mut self.framebuf);

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
                        load:  wgpu::LoadOp::Clear(wgpu::Color { r: 0.1, g: 0.1, b: 0.18, a: 1.0 }),
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

// ── application ───────────────────────────────────────────────────────────────

struct App {
    shell:     String,
    font:      Font,
    gpu:       Option<Gpu>,
    window:    Option<Arc<Window>>,
    pty:       Option<Pty>,
    pane:      Option<Pane>,
    font_px:   f32,
    cell_w:    usize,
    cell_h:    usize,
    baseline:  usize,
    modifiers: ModifiersState,
}

impl App {
    fn new(shell: &str) -> Self {
        let font = Font::from_bytes(FONT_BYTES, FontSettings::default()).expect("font");
        Self {
            shell:     shell.to_owned(),
            font,
            gpu:       None,
            window:    None,
            pty:       None,
            pane:      None,
            font_px:   FONT_SIZE,
            cell_w:    8,
            cell_h:    16,
            baseline:  13,
            modifiers: ModifiersState::default(),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attrs = WindowAttributes::default()
            .with_title("rusty")
            .with_inner_size(winit::dpi::LogicalSize::new(1024u32, 768u32));
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));

        let scale    = window.scale_factor() as f32;
        self.font_px = FONT_SIZE * scale;
        (self.cell_w, self.cell_h, self.baseline) = measure_cell(&self.font, self.font_px);
        tracing::info!("scale={scale} font_px={} cell={}×{} baseline={}", self.font_px, self.cell_w, self.cell_h, self.baseline);

        let phys = window.inner_size();
        let cols = (phys.width  as usize / self.cell_w).max(1);
        let rows = (phys.height as usize / self.cell_h).max(1);

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
                let rows = (phys.height as usize / self.cell_h).max(1);
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
                if let Some(pane) = &mut self.pane {
                    if lines < 0.0 {
                        pane.scroll_up_view((-lines).ceil() as usize);
                    } else {
                        pane.scroll_down_view(lines.ceil() as usize);
                    }
                }
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }


            WindowEvent::KeyboardInput {
                event: KeyEvent { logical_key, text, state: ElementState::Pressed, .. }, ..
            } => {
                let ctrl = self.modifiers.control_key();

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
                    // No ctrl — use the OS-composed text first (handles layout, shift, option).
                    // Fall through to logical_key for named keys that produce no text.
                    if let Some(t) = &text {
                        Some(t.as_str().as_bytes().to_vec())
                    } else {
                        match &logical_key {
                            Key::Named(NamedKey::Enter)        => Some(b"\r".to_vec()),
                            Key::Named(NamedKey::Backspace)    => Some(b"\x7f".to_vec()),
                            Key::Named(NamedKey::Escape)       => Some(b"\x1b".to_vec()),
                            Key::Named(NamedKey::Tab)          => Some(b"\t".to_vec()),
                            Key::Named(NamedKey::ArrowUp)      => Some(b"\x1b[A".to_vec()),
                            Key::Named(NamedKey::ArrowDown)    => Some(b"\x1b[B".to_vec()),
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
                        }
                    }
                };

                if let Some(b) = bytes {
                    // Any keystroke snaps back to live view.
                    if let Some(pane) = &mut self.pane {
                        pane.scroll_off = 0;
                    }
                    if let Some(pty) = &mut self.pty {
                        let _ = pty.write_bytes(&b);
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                // Drain PTY output into pane.
                if let (Some(pty), Some(pane)) = (&self.pty, &mut self.pane) {
                    while let Ok(bytes) = pty.rx.try_recv() {
                        pane.process(&bytes);
                    }
                }
                if let (Some(gpu), Some(pane)) = (&mut self.gpu, &self.pane) {
                    gpu.render(pane, &self.font, self.font_px, self.cell_w, self.cell_h, self.baseline);
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
    pub fn run(shell: &str) {
        #[cfg(target_os = "macos")]
        let event_loop = {
            let mut builder = EventLoop::builder();
            builder.with_activation_policy(ActivationPolicy::Regular);
            builder.build().expect("event loop")
        };
        #[cfg(not(target_os = "macos"))]
        let event_loop = EventLoop::new().expect("event loop");

        event_loop.set_control_flow(ControlFlow::Poll);
        let mut app = App::new(shell);
        event_loop.run_app(&mut app).expect("event loop error");
    }
}

// ── software rasterizer ───────────────────────────────────────────────────────

fn paint_framebuf(
    pane:     &Pane,
    font:     &Font,
    font_px:  f32,
    cell_w:   usize,
    cell_h:   usize,
    baseline: usize,
    fb_w:     usize,
    fb_h:     usize,
    buf:      &mut [u8],
) {
    let bg_default = Color::Default.to_rgba(false);
    buf.chunks_exact_mut(4).for_each(|p| p.copy_from_slice(&bg_default));

    let grid       = &pane.grid;
    let cursor     = pane.cursor;
    let scroll_off = pane.scroll_off;
    let total      = grid.total_rows();
    // First scrollback row to show = total - height - scroll_off
    let view_start = total.saturating_sub(grid.height).saturating_sub(scroll_off);
    let scrolling  = scroll_off > 0;

    for screen_row in 0..grid.height {
        let src_row = view_start + screen_row;
        for col in 0..grid.width {
            let cell = *grid.scrollback_get(col, src_row);

            // Cursor only shown when at live bottom and not scrolled away.
            let is_cursor = !scrolling
                && col == cursor.col
                && screen_row == cursor.row
                && cursor.visible;

            let fg: [u8; 4] = if is_cursor { cell.bg.to_rgba(false) } else { cell.fg.to_rgba(true) };
            let bg: [u8; 4] = if is_cursor { [0xcc, 0xcc, 0xcc, 0xff] } else { cell.bg.to_rgba(false) };

            let px = col * cell_w;
            let py = screen_row * cell_h;

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
