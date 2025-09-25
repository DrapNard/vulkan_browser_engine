use vulkan_browser_engine::pwa::{manifest::Manifest, PwaRuntime};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = PwaRuntime::new().await?;

    let manifest_content = tokio::fs::read_to_string("app/manifest.json").await?;
    let manifest: Manifest = serde_json::from_str(&manifest_content)?;

    let app_id = runtime.install_app(&manifest).await?;
    println!("Installed PWA with ID: {}", app_id);

    // register_service_worker takes (&str, Option<&str>)
    let worker_id = runtime
        .register_service_worker("app/sw.js", Some("/"))
        .await?;
    println!("Registered Service Worker: {}", worker_id);

    let usage = runtime.get_total_storage_usage().await?;
    println!("Total storage usage: {} bytes", usage.total_size);

    runtime.shutdown().await?;
    Ok(())
}
