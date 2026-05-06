use winit::dpi::PhysicalSize;
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
        .with_surface_size(PhysicalSize::new(width, height))
        .with_min_surface_size(PhysicalSize::new(640, 400))
        .with_transparent(false)
        .with_resizable(true)
        .with_decorations(decorations);

    event_loop.create_window(attrs).unwrap()
}
