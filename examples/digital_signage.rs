use tokio::time::{sleep, Duration};
use vulkan_browser_engine::{BrowserConfig as Config, BrowserEngine};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config {
        enable_security_features: true,
        enable_sandbox: true,
        ..Default::default()
    };

    let engine = BrowserEngine::new(config).await?;

    let playlist = [
        "https://signage.company.com/slide1",
        "https://signage.company.com/slide2",
        "https://signage.company.com/slide3",
    ];
    let mut i = 0usize;

    loop {
        engine.load_url(playlist[i]).await?;
        sleep(Duration::from_secs(20)).await;
        i = (i + 1) % playlist.len();
    }
}
