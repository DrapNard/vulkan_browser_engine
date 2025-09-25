use vulkan_browser_engine::{BrowserConfig as Config, BrowserEngine};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config {
        enable_security_features: true,
        enable_sandbox: true,
        ..Default::default()
    };

    let engine = BrowserEngine::new(config).await?;
    engine.load_url("https://google.com").await?;

    // Keep alive; the hosting app controls lifecycle.
    futures::future::pending::<()>().await;
    Ok(())
}
