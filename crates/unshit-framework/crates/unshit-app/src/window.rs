use winit::dpi::PhysicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

pub fn create_window(
    event_loop: &dyn ActiveEventLoop,
    title: &str,
    width: u32,
    height: u32,
    decorations: bool,
    visible: bool,
) -> Box<dyn Window> {
    let attrs = WindowAttributes::default()
        .with_title(title)
        .with_surface_size(PhysicalSize::new(width, height))
        .with_transparent(false)
        .with_decorations(decorations)
        // A window mapped before the GPU is ready is not an early window, it
        // is a hung one: adapter and device creation run on this thread, so
        // nothing pumps messages until they finish. Callers that would rather
        // wait and appear drawn ask for `visible: false` here.
        .with_visible(visible);

    event_loop.create_window(attrs).unwrap()
}
