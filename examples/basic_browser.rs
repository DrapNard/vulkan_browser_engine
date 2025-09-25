use vulkan_browser_engine::{BrowserConfig as Config, BrowserEngine};
use winit::{event::Event, event_loop::EventLoop, window::WindowBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let event_loop = EventLoop::new()?;
    let window = WindowBuilder::new()
        .with_title("Vulkan Browser")
        .with_inner_size(winit::dpi::LogicalSize::new(1280, 720))
        .build(&event_loop)?;

    let config = Config::default();
    let engine = BrowserEngine::new(config).await?;

    engine.load_url("https://example.com").await?;

    event_loop.run(move |event, elwt| match event {
        Event::WindowEvent { window_id, event } if window_id == window.id() => match event {
            winit::event::WindowEvent::CloseRequested => elwt.exit(),
            winit::event::WindowEvent::RedrawRequested => {}
            _ => {}
        },
        Event::AboutToWait => {
            window.request_redraw();
        }
        _ => {}
    })?;

    Ok(())
}
