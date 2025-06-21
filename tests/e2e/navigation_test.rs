use vulkan_renderer::{BrowserEngine, Config};
use tokio_test;

#[tokio::test]
async fn test_basic_navigation() {
    let config = Config::default();
    let mut engine = BrowserEngine::new(config, None).unwrap();
    
    let result = engine.load_url("https://example.com").await;
    assert!(result.is_ok());
    
    let history = engine.get_navigation_history();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0], "https://example.com");
}

#[tokio::test]
async fn test_navigation_history() {
    let config = Config::default();
    let mut engine = BrowserEngine::new(config, None).unwrap();
    
    engine.load_url("https://example.com").await.unwrap();
    engine.load_url("https://google.com").await.unwrap();
    engine.load_url("https://github.com").await.unwrap();
    
    let history = engine.get_navigation_history();
    assert_eq!(history.len(), 3);
    assert_eq!(history[2], "https://github.com");
    
    engine.navigate_back().await.unwrap();
    let current_url = engine.get_current_url();
    assert_eq!(current_url, "https://google.com");
    
    engine.navigate_forward().await.unwrap();
    let current_url = engine.get_current_url();
    assert_eq!(current_url, "https://github.com");
}

#[tokio::test]
async fn test_reload_functionality() {
    let config = Config::default();
    let mut engine = BrowserEngine::new(config, None).unwrap();
    
    engine.load_url("https://example.com").await.unwrap();
    let original_load_time = engine.get_load_time();
    
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    
    engine.reload().await.unwrap();
    let new_load_time = engine.get_load_time();
    
    assert!(new_load_time > original_load_time);
}

#[tokio::test]
async fn test_invalid_url_handling() {
    let config = Config::default();
    let mut engine = BrowserEngine::new(config, None).unwrap();
    
    let result = engine.load_url("invalid-url").await;
    assert!(result.is_err());
    
    let result = engine.load_url("").await;
    assert!(result.is_err());
    
    let result = engine.load_url("ftp://invalid-protocol.com").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_stop_navigation() {
    let config = Config::default();
    let mut engine = BrowserEngine::new(config, None).unwrap();
    
    let load_future = engine.load_url("https://slow-website.com");
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    engine.stop_loading();
    
    let result = load_future.await;
    assert!(result.is_err() || engine.get_loading_state() == LoadingState::Stopped);
}

#[tokio::test]
async fn test_concurrent_navigation() {
    let config = Config::default();
    let mut engine = BrowserEngine::new(config, None).unwrap();
    
    let load1 = engine.load_url("https://example1.com");
    let load2 = engine.load_url("https://example2.com");
    
    let (result1, result2) = tokio::join!(load1, load2);
    
    assert!(result1.is_ok() || result2.is_ok());
    let final_url = engine.get_current_url();
    assert!(final_url == "https://example1.com" || final_url == "https://example2.com");
}