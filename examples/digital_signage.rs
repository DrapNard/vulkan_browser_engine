use vulkan_browser_engine::{BrowserEngine, BrowserConfig as Config};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config {
        display_mode: DisplayMode::DigitalSignage,
        auto_reload_interval: Some(Duration::from_hours(6)),
        power_management: true,
        ..Default::default()
    };
    
    let mut engine = BrowserEngine::new(config, None)?;
    
    let playlist = vec![
        "https://signage.company.com/slide1",
        "https://signage.company.com/slide2", 
        "https://signage.company.com/slide3",
    ];
    
    let mut current_slide = 0;
    let slide_duration = Duration::from_secs(30);
    let mut last_switch = std::time::Instant::now();
    
    engine.load_url(&playlist[current_slide])?;
    
    loop {
        if last_switch.elapsed() >= slide_duration {
            current_slide = (current_slide + 1) % playlist.len();
            engine.load_url(&playlist[current_slide])?;
            last_switch = std::time::Instant::now();
        }
        
        engine.process_events()?;
        engine.render_frame()?;
        std::thread::sleep(Duration::from_millis(16));
    }
}