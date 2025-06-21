use vulkan_renderer::pwa::*;
use tokio_test;

#[tokio::test]
async fn test_full_pwa_lifecycle() {
    let mut runtime = PwaRuntime::new().await.unwrap();
    
    let manifest = create_test_manifest();
    let app_id = runtime.install_app(&manifest).await.unwrap();
    
    assert!(!app_id.is_empty());
    
    let sw_script = create_test_service_worker();
    let worker_id = runtime.register_service_worker(&sw_script).await.unwrap();
    
    assert!(!worker_id.is_empty());
    
    test_caching_strategies(&mut runtime, &app_id).await;
    test_offline_capability(&mut runtime, &app_id).await;
    test_background_sync(&mut runtime, &app_id).await;
    
    runtime.uninstall_app(&app_id).await.unwrap();
    
    let installed_apps = runtime.get_installed_apps().await;
    assert!(installed_apps.iter().find(|app| app.id == app_id).is_none());
}

async fn test_caching_strategies(runtime: &mut PwaRuntime, app_id: &str) {
    let cache_strategies = vec![
        ("cache-first", CacheStrategy::CacheFirst),
        ("network-first", CacheStrategy::NetworkFirst),
        ("stale-while-revalidate", CacheStrategy::StaleWhileRevalidate),
    ];
    
    for (strategy_name, strategy) in cache_strategies {
        let cache_name = format!("{}-{}", app_id, strategy_name);
        
        let request = FetchRequest {
            url: format!("https://example.com/{}", strategy_name),
            method: "GET".to_string(),
            headers: std::collections::HashMap::new(),
            body: None,
        };
        
        let response = FetchResponse {
            status: 200,
            headers: std::collections::HashMap::new(),
            body: format!("Response for {}", strategy_name).into_bytes(),
        };
        
        runtime.cache_manager.write().await.add_to_cache(&cache_name, &request.url, &response).await.unwrap();
        
        let cached_response = runtime.cache_manager.read().await.match_request(&request).await.unwrap();
        assert!(cached_response.is_some());
        assert_eq!(cached_response.unwrap().status, 200);
    }
}

async fn test_offline_capability(runtime: &mut PwaRuntime, app_id: &str) {
    let offline_page_request = FetchRequest {
        url: "https://example.com/offline.html".to_string(),
        method: "GET".to_string(),
        headers: std::collections::HashMap::new(),
        body: None,
    };
    
    let offline_page_response = FetchResponse {
        status: 200,
        headers: [("content-type".to_string(), "text/html".to_string())].iter().cloned().collect(),
        body: b"<html><body><h1>You're offline</h1></body></html>".to_vec(),
    };
    
    runtime.cache_manager.write().await.add_to_cache(
        &format!("{}-offline", app_id), 
        &offline_page_request.url, 
        &offline_page_response
    ).await.unwrap();
    
    let response = runtime.handle_fetch_request(&offline_page_request).await.unwrap();
    assert_eq!(response.status, 200);
    assert!(String::from_utf8_lossy(&response.body).contains("You're offline"));
}

async fn test_background_sync(runtime: &mut PwaRuntime, app_id: &str) {
    let sync_data = serde_json::json!({
        "type": "background_sync",
        "data": {
            "message": "Test background sync",
            "timestamp": chrono::Utc::now().timestamp()
        }
    });
    
    runtime.storage_manager.write().await.set_local_storage(
        app_id, 
        "pending_sync", 
        &sync_data.to_string()
    ).await.unwrap();
    
    let stored_data = runtime.storage_manager.read().await.get_local_storage(app_id, "pending_sync");
    assert!(stored_data.is_some());
    
    let parsed_data: serde_json::Value = serde_json::from_str(&stored_data.unwrap()).unwrap();
    assert_eq!(parsed_data["type"], "background_sync");
}

fn create_test_manifest() -> Manifest {
    Manifest {
        name: "Integration Test PWA".to_string(),
        short_name: Some("TestPWA".to_string()),
        description: Some("A PWA for integration testing".to_string()),
        start_url: "/".to_string(),
        scope: Some("/".to_string()),
        display: DisplayMode::Standalone,
        orientation: Some(Orientation::Any),
        theme_color: Some("#2196F3".to_string()),
        background_color: Some("#FFFFFF".to_string()),
        icons: vec![
            Icon {
                src: "/icon-192.png".to_string(),
                sizes: Some("192x192".to_string()),
                icon_type: Some("image/png".to_string()),
                purpose: Some(IconPurpose::Any),
            },
            Icon {
                src: "/icon-512.png".to_string(),
                sizes: Some("512x512".to_string()),
                icon_type: Some("image/png".to_string()),
                purpose: Some(IconPurpose::Maskable),
            },
        ],
        service_worker: Some("/sw.js".to_string()),
        categories: vec!["productivity".to_string(), "utilities".to_string()],
        screenshots: vec![
            Screenshot {
                src: "/screenshot1.png".to_string(),
                sizes: Some("1280x720".to_string()),
                screenshot_type: Some("image/png".to_string()),
                form_factor: Some(FormFactor::Wide),
                label: Some("Main interface".to_string()),
            }
        ],
        ..Default::default()
    }
}

fn create_test_service_worker() -> String {
    r#"
    const CACHE_NAME = 'test-pwa-v1';
    const urlsToCache = [
        '/',
        '/styles/main.css',
        '/scripts/main.js',
        '/offline.html'
    ];

    self.addEventListener('install', function(event) {
        event.waitUntil(
            caches.open(CACHE_NAME)
                .then(function(cache) {
                    return cache.addAll(urlsToCache);
                })
        );
    });

    self.addEventListener('fetch', function(event) {
        event.respondWith(
            caches.match(event.request)
                .then(function(response) {
                    if (response) {
                        return response;
                    }
                    return fetch(event.request);
                }
            )
        );
    });

    self.addEventListener('activate', function(event) {
        event.waitUntil(
            caches.keys().then(function(cacheNames) {
                return Promise.all(
                    cacheNames.map(function(cacheName) {
                        if (cacheName !== CACHE_NAME) {
                            return caches.delete(cacheName);
                        }
                    })
                );
            })
        );
    });

    self.addEventListener('sync', function(event) {
        if (event.tag === 'background-sync') {
            event.waitUntil(doBackgroundSync());
        }
    });

    function doBackgroundSync() {
        return fetch('/api/sync', {
            method: 'POST',
            body: JSON.stringify({
                timestamp: Date.now(),
                data: 'background sync data'
            })
        });
    }
    "#.to_string()
}