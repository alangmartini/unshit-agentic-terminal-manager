use winit::dpi::LogicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

pub fn create_window(
    event_loop: &dyn ActiveEventLoop,
    title: &str,
    width: u32,
    height: u32,
    decorations: bool,
) -> Box<dyn Window> {
    let attrs = WindowAttributes::default()
        .with_title(title)
        .with_surface_size(LogicalSize::new(width, height))
        .with_decorations(decorations);

    event_loop.create_window(attrs).unwrap()
}
