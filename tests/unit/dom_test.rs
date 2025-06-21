use vulkan_renderer::core::dom::*;

#[tokio::test]
async fn test_dom_creation() {
    let doc = Document::new();
    assert_eq!(doc.get_nodes().len(), 0);
}

#[tokio::test]
async fn test_element_insertion() {
    let mut doc = Document::new();
    let element = Element::new("div".to_string());
    let node_id = doc.insert_element(element);
    
    assert_eq!(doc.get_nodes().len(), 1);
    assert!(doc.get_element(node_id).is_some());
}

#[tokio::test]
async fn test_html_parsing() {
    let html = r#"
        <html>
            <body>
                <div id="test">Hello World</div>
            </body>
        </html>
    "#;
    
    let doc = Document::parse(html).unwrap();
    assert!(doc.get_nodes().len() > 0);
}

tests/unit/js_engine_test.rs
use vulkan_renderer::js_engine::*;

#[tokio::test]
async fn test_js_execution() {
    let mut engine = JsEngine::new().await.unwrap();
    let result = engine.execute("2 + 2").await.unwrap();
    
    assert_eq!(result, serde_json::Value::Number(serde_json::Number::from(4)));
}

#[tokio::test]
async fn test_console_log() {
    let mut engine = JsEngine::new().await.unwrap();
    let result = engine.execute("console.log('Hello World'); 42").await;
    
    assert!(result.is_ok());
}

tests/unit/renderer_test.rs
use vulkan_renderer::renderer::*;

#[tokio::test]
async fn test_renderer_creation() {
    // Note: This test would require a headless Vulkan setup
    // let renderer = VulkanRenderer::new().await;
    // assert!(renderer.is_ok());
}