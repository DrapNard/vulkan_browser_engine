use vulkan_renderer::pwa::{PwaRuntime, Manifest};
use std::path::Path;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = PwaRuntime::new().await?;
    
    let manifest_content = tokio::fs::read_to_string("app/manifest.json").await?;
    let manifest: Manifest = serde_json::from_str(&manifest_content)?;
    
    let app_id = runtime.install_app(&manifest).await?;
    println!("Installed PWA with ID: {}", app_id);
    
    let sw_script = tokio::fs::read_to_string("app/sw.js").await?;
    let worker_id = runtime.register_service_worker(&sw_script).await?;
    println!("Registered Service Worker: {}", worker_id);
    
    runtime.run().await?;
    
    Ok(())
}