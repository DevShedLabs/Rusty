use std::sync::Arc;

use fontdue::{Font, FontSettings};
use rusty_core::Color;
use rusty_mux::pane::Pane;
use rusty_pty::{Pty, PtySize};
use winit::{
    event::{ElementState, Event, KeyEvent, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::WindowBuilder,
};

const FONT_SIZE: f32 = 14.0;
const FONT_BYTES: &[u8] = include_bytes!("../assets/JetBrainsMono-Regular.ttf");

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

pub struct TerminalWindow;

impl TerminalWindow {
    pub fn run(shell: &str) {
        let event_loop = EventLoop::new().expect("event loop");
        let window = Arc::new(
            WindowBuilder::new()
                .with_title("rusty")
                .with_inner_size(winit::dpi::LogicalSize::new(1024u32, 768u32))
                .build(&event_loop)
                .expect("window"),
        );

        // ── wgpu bootstrap ──────────────────────────────────────────────────
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..Default::default()
        });
        let surface = instance.create_surface(window.clone()).expect("surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        ))
        .expect("device");

        let size = window.inner_size();
        let surface_format = wgpu::TextureFormat::Bgra8Unorm;
        let mut surface_config = wgpu::SurfaceConfiguration {
            usage:        wgpu::TextureUsages::RENDER_ATTACHMENT,
            format:       surface_format,
            width:        size.width,
            height:       size.height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode:   wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        // ── font ────────────────────────────────────────────────────────────
        let font = Font::from_bytes(FONT_BYTES, FontSettings::default()).expect("font");
        let (metrics, _) = font.rasterize('M', FONT_SIZE);
        let cell_w = metrics.advance_width.ceil() as usize;
        let cell_h = (FONT_SIZE * 1.4).ceil() as usize;
        let baseline = (FONT_SIZE * 1.1).ceil() as usize;

        // ── terminal sizing ─────────────────────────────────────────────────
        let cols = (size.width as usize / cell_w).max(1);
        let rows = (size.height as usize / cell_h).max(1);

        // ── PTY + pane ──────────────────────────────────────────────────────
        let pty_size = PtySize {
            cols: cols as u16,
            rows: rows as u16,
            px_w: size.width as u16,
            px_h: size.height as u16,
        };
        let mut pty = Pty::spawn(shell, pty_size).expect("pty spawn");
        let mut pane = Pane::new(0, cols, rows);

        // ── screen texture (BGRA software blit target) ──────────────────────
        let mut fb_width  = size.width  as usize;
        let mut fb_height = size.height as usize;
        let mut framebuf  = vec![0u8; fb_width * fb_height * 4];

        let texture_desc = wgpu::TextureDescriptor {
            label:           Some("screen"),
            size:            wgpu::Extent3d { width: fb_width as u32, height: fb_height as u32, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          wgpu::TextureFormat::Rgba8Unorm,
            usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats:    &[],
        };
        let mut screen_tex  = device.create_texture(&texture_desc);
        let screen_view     = screen_tex.create_view(&wgpu::TextureViewDescriptor::default());

        // ── fullscreen blit pipeline ─────────────────────────────────────────
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("blit_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding:    0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty:         wgpu::BindingType::Texture {
                        sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled:   false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding:    1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty:         wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let mut bind_group = make_bind_group(&device, &bgl, &sampler, &screen_view);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:                Some("blit_pl"),
            bind_group_layouts:   &[&bgl],
            push_constant_ranges: &[],
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:         Some("blit"),
            layout:        Some(&pipeline_layout),
            vertex:        wgpu::VertexState { module: &blit_shader, entry_point: "vs_main", buffers: &[], compilation_options: Default::default() },
            fragment:      Some(wgpu::FragmentState {
                module:            &blit_shader,
                entry_point:       "fs_main",
                targets:           &[Some(wgpu::ColorTargetState {
                    format:     surface_format,
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

        // ── event loop ───────────────────────────────────────────────────────
        event_loop.set_control_flow(ControlFlow::Poll);

        event_loop
            .run(move |event, target| {
                match event {
                    Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                        target.exit();
                    }

                    Event::WindowEvent { event: WindowEvent::Resized(new_size), .. } => {
                        if new_size.width == 0 || new_size.height == 0 { return; }
                        surface_config.width  = new_size.width;
                        surface_config.height = new_size.height;
                        surface.configure(&device, &surface_config);

                        fb_width  = new_size.width  as usize;
                        fb_height = new_size.height as usize;
                        framebuf  = vec![0u8; fb_width * fb_height * 4];

                        let new_cols = (fb_width  / cell_w).max(1);
                        let new_rows = (fb_height / cell_h).max(1);
                        pane = Pane::new(0, new_cols, new_rows);
                        let _ = pty.resize(PtySize {
                            cols: new_cols as u16,
                            rows: new_rows as u16,
                            px_w: new_size.width as u16,
                            px_h: new_size.height as u16,
                        });

                        screen_tex  = device.create_texture(&wgpu::TextureDescriptor {
                            size: wgpu::Extent3d { width: new_size.width, height: new_size.height, depth_or_array_layers: 1 },
                            ..texture_desc
                        });
                        let view   = screen_tex.create_view(&wgpu::TextureViewDescriptor::default());
                        bind_group = make_bind_group(&device, &bgl, &sampler, &view);
                    }

                    Event::WindowEvent {
                        event: WindowEvent::KeyboardInput { event: KeyEvent { logical_key, state: ElementState::Pressed, .. }, .. },
                        ..
                    } => {
                        let bytes: Option<Vec<u8>> = match &logical_key {
                            Key::Character(s) => Some(s.as_str().as_bytes().to_vec()),
                            Key::Named(NamedKey::Enter)     => Some(b"\r".to_vec()),
                            Key::Named(NamedKey::Backspace) => Some(b"\x7f".to_vec()),
                            Key::Named(NamedKey::Escape)    => Some(b"\x1b".to_vec()),
                            Key::Named(NamedKey::Tab)       => Some(b"\t".to_vec()),
                            Key::Named(NamedKey::ArrowUp)   => Some(b"\x1b[A".to_vec()),
                            Key::Named(NamedKey::ArrowDown) => Some(b"\x1b[B".to_vec()),
                            Key::Named(NamedKey::ArrowRight)=> Some(b"\x1b[C".to_vec()),
                            Key::Named(NamedKey::ArrowLeft) => Some(b"\x1b[D".to_vec()),
                            _ => None,
                        };
                        if let Some(b) = bytes {
                            let _ = pty.write_bytes(&b);
                        }
                    }

                    Event::AboutToWait => {
                        // Drain all pending PTY output
                        while let Ok(bytes) = pty.rx.try_recv() {
                            pane.process(&bytes);
                        }
                        window.request_redraw();
                    }

                    Event::WindowEvent { event: WindowEvent::RedrawRequested, .. } => {
                        // ── rasterize grid into framebuf ─────────────────────
                        let bg_default = Color::Default.to_rgba(false);
                        framebuf.chunks_exact_mut(4).for_each(|p| p.copy_from_slice(&bg_default));

                        let grid = &pane.grid;
                        let cursor = pane.cursor;

                        for row in 0..grid.height {
                            for col in 0..grid.width {
                                let cell = grid.get(col, row);
                                let is_cursor = col == cursor.col && row == cursor.row && cursor.visible;

                                let fg_rgba = if is_cursor {
                                    cell.bg.to_rgba(false)
                                } else {
                                    cell.fg.to_rgba(true)
                                };
                                let bg_rgba = if is_cursor {
                                    [0xaa, 0xaa, 0xaa, 0xff]
                                } else {
                                    cell.bg.to_rgba(false)
                                };

                                let px = col * cell_w;
                                let py = row * cell_h;

                                // Fill cell background
                                for dy in 0..cell_h {
                                    for dx in 0..cell_w {
                                        let x = px + dx;
                                        let y = py + dy;
                                        if x < fb_width && y < fb_height {
                                            let i = (y * fb_width + x) * 4;
                                            framebuf[i..i + 4].copy_from_slice(&bg_rgba);
                                        }
                                    }
                                }

                                // Rasterize glyph
                                if cell.ch != ' ' {
                                    let (metrics, bitmap) = font.rasterize(cell.ch, FONT_SIZE);
                                    let gx = px as i32 + metrics.xmin;
                                    let gy = py as i32 + baseline as i32 - metrics.height as i32 - metrics.ymin;
                                    for by in 0..metrics.height {
                                        for bx in 0..metrics.width {
                                            let alpha = bitmap[by * metrics.width + bx];
                                            if alpha == 0 { continue; }
                                            let x = gx + bx as i32;
                                            let y = gy + by as i32;
                                            if x < 0 || y < 0 { continue; }
                                            let (x, y) = (x as usize, y as usize);
                                            if x >= fb_width || y >= fb_height { continue; }
                                            let i = (y * fb_width + x) * 4;
                                            let a = alpha as u32;
                                            let blend = |fg: u8, bg: u8| -> u8 {
                                                ((fg as u32 * a + bg as u32 * (255 - a)) / 255) as u8
                                            };
                                            framebuf[i]     = blend(fg_rgba[0], bg_rgba[0]);
                                            framebuf[i + 1] = blend(fg_rgba[1], bg_rgba[1]);
                                            framebuf[i + 2] = blend(fg_rgba[2], bg_rgba[2]);
                                            framebuf[i + 3] = 0xff;
                                        }
                                    }
                                }
                            }
                        }

                        // ── upload framebuf → texture ────────────────────────
                        queue.write_texture(
                            screen_tex.as_image_copy(),
                            &framebuf,
                            wgpu::ImageDataLayout {
                                offset:         0,
                                bytes_per_row:  Some((fb_width * 4) as u32),
                                rows_per_image: Some(fb_height as u32),
                            },
                            wgpu::Extent3d { width: fb_width as u32, height: fb_height as u32, depth_or_array_layers: 1 },
                        );

                        // ── blit texture → surface ───────────────────────────
                        let frame = match surface.get_current_texture() {
                            Ok(f) => f,
                            Err(_) => return,
                        };
                        let frame_view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
                        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                        {
                            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label:                    Some("blit"),
                                color_attachments:        &[Some(wgpu::RenderPassColorAttachment {
                                    view:           &frame_view,
                                    resolve_target: None,
                                    ops:            wgpu::Operations {
                                        load:  wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes:         None,
                                occlusion_query_set:      None,
                            });
                            pass.set_pipeline(&blit_pipeline);
                            pass.set_bind_group(0, &bind_group, &[]);
                            pass.draw(0..3, 0..1);
                        }
                        queue.submit(std::iter::once(encoder.finish()));
                        frame.present();
                    }

                    _ => {}
                }
            })
            .expect("event loop error");
    }
}

// Fullscreen triangle blit — renders a texture covering the whole viewport.
const BLIT_SHADER: &str = r#"
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

struct Vout { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> Vout {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var uv = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0, -1.0),
    );
    var out: Vout;
    out.pos = vec4<f32>(pos[vi], 0.0, 1.0);
    out.uv  = uv[vi];
    return out;
}

@fragment
fn fs_main(in: Vout) -> @location(0) vec4<f32> {
    return textureSample(t, s, in.uv);
}
"#;
