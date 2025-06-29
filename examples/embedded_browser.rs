use vulkan_browser_engine::{BrowserEngine, BrowserConfig as Config};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config {
        embedded_mode: EmbeddedMode::Kiosk,
        allowed_origins: vec!["https://dashboard.company.com".to_string()],
        disable_navigation: true,
        ..Default::default()
    };
    
    let mut engine = BrowserEngine::new(config, None)?;
    
    engine.load_url("https://dashboard.company.com")?;
    
    engine.set_fullscreen(true)?;
    engine.disable_context_menu()?;
    engine.hide_cursor_after(std::time::Duration::from_secs(10))?;
    
    loop {
        engine.process_events()?;
        engine.render_frame()?;
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}