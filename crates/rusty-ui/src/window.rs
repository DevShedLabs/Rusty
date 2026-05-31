use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};

pub struct TerminalWindow;

impl TerminalWindow {
    pub fn run() {
        let event_loop = EventLoop::new().expect("failed to create event loop");
        let window = WindowBuilder::new()
            .with_title("rusty")
            .with_inner_size(winit::dpi::LogicalSize::new(1024u32, 768u32))
            .build(&event_loop)
            .expect("failed to create window");

        event_loop.set_control_flow(ControlFlow::Wait);

        event_loop
            .run(move |event, target| match event {
                Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                    target.exit();
                }
                Event::WindowEvent { event: WindowEvent::Resized(size), .. } => {
                    tracing::debug!("resized to {}x{}", size.width, size.height);
                }
                Event::WindowEvent { event: WindowEvent::RedrawRequested, .. } => {
                    window.request_redraw();
                }
                _ => {}
            })
            .expect("event loop error");
    }
}
