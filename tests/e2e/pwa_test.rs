use vulkan_renderer::pwa::*;
use tokio_test;

#[tokio::test]
async fn test_pwa_installation() {
    let mut runtime = PwaRuntime::new().await.unwrap();
    
    let manifest = Manifest {
        name: "Test PWA".to_string(),
        short_name: Some("TestPWA".to_string()),
        start_url: "/".to_string(),
        display: DisplayMode::Standalone,
        theme_color: Some("#000000".to_string()),
        background_color: Some("#FFFFFF".to_string()),
        icons: vec![
            Icon {
                src: "/icon-192.png".to_string(),
                sizes: Some("192x192".to_string()),
                icon_type: Some("image/png".to_string()),
                purpose: Some(IconPurpose::Any),
            }
        ],
        service_worker: Some("/sw.js".to_string()),
        ..Default::default()
    };
    
    let app_id = runtime.install_app(&manifest).await.unwrap();
    assert!(!app_id.is_empty());
    
    let installed_apps = runtime.get_installed_apps().await;
    assert_eq!(installed_apps.len(), 1);
    assert_eq!(installed_apps[0].manifest.name, "Test PWA");
}

#[tokio::test]
async fn test_service_worker_registration() {
    let mut runtime = PwaRuntime::new().await.unwrap();
    
    let sw_script = r#"
        self.addEventListener('install', function(event) {
            console.log('Service Worker installing');
        });
        
        self.addEventListener('activate', function(event) {
            console.log('Service Worker activated');
        });
        
        self.addEventListener('fetch', function(event) {
            event.respondWith(fetch(event.request));
        });
    "#;
    
    let worker_id = runtime.register_service_worker(sw_script).await.unwrap();
    assert!(!worker_id.is_empty());
}

#[tokio::test]
async fn test_cache_api() {
    let mut runtime = PwaRuntime::new().await.unwrap();
    
    let request = FetchRequest {
        url: "https://example.com/test.json".to_string(),
        method: "GET".to_string(),
        headers: std::collections::HashMap::new(),
        body: None,
    };
    
    let response = FetchResponse {
        status: 200,
        headers: std::collections::HashMap::new(),
        body: b"test data".to_vec(),
    };
    
    runtime.cache_manager.write().await.add_to_cache("test-cache", &request.url, &response).await.unwrap();
    
    let cached_response = runtime.cache_manager.read().await.match_request(&request).await.unwrap();
    assert!(cached_response.is_some());
    assert_eq!(cached_response.unwrap().status, 200);
}

#[tokio::test]
async fn test_offline_functionality() {
    let mut runtime = PwaRuntime::new().await.unwrap();
    
    let offline_request = FetchRequest {
        url: "https://offline-site.com/data".to_string(),
        method: "GET".to_string(),
        headers: std::collections::HashMap::new(),
        body: None,
    };
    
    let cached_response = FetchResponse {
        status: 200,
        headers: std::collections::HashMap::new(),
        body: b"cached offline data".to_vec(),
    };
    
    runtime.cache_manager.write().await.add_to_cache("offline-cache", &offline_request.url, &cached_response).await.unwrap();
    
    let response = runtime.handle_fetch_request(&offline_request).await.unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"cached offline data");
}

#[tokio::test]
async fn test_storage_quota_management() {
    let mut runtime = PwaRuntime::new().await.unwrap();
    let app_id = "test_app_123";
    
    let usage = runtime.get_app_storage_usage(app_id).await.unwrap();
    assert_eq!(usage.cache_size, 0);
    assert_eq!(usage.indexeddb_size, 0);
    assert_eq!(usage.local_storage_size, 0);
    
    runtime.storage_manager.write().await.set_local_storage(app_id, "test_key", "test_value").await.unwrap();
    
    let updated_usage = runtime.get_app_storage_usage(app_id).await.unwrap();
    assert!(updated_usage.local_storage_size > 0);
}

#[tokio::test]
async fn test_manifest_parsing() {
    let manifest_json = r#"
    {
        "name": "Test Application",
        "short_name": "TestApp",
        "start_url": "/index.html",
        "display": "standalone",
        "theme_color": "#2196F3",
        "background_color": "#FFFFFF",
        "icons": [
            {
                "src": "/icon-192.png",
                "sizes": "192x192",
                "type": "image/png"
            }
        ]
    }
    "#;
    
    let parser = ManifestParser::new();
    let manifest = parser.parse(manifest_json, Some("https://example.com")).unwrap();
    
    assert_eq!(manifest.name, "Test Application");
    assert_eq!(manifest.short_name, Some("TestApp".to_string()));
    assert_eq!(manifest.start_url, "https://example.com/index.html");
    assert_eq!(manifest.icons.len(), 1);
}
