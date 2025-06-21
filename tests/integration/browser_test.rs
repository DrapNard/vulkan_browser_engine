use vulkan_renderer::{BrowserEngine, Config};
use tokio_test;

#[tokio::test]
async fn test_full_page_rendering() {
    let config = Config::default();
    let mut engine = BrowserEngine::new(config, None).unwrap();
    
    let html = r#"
        <!DOCTYPE html>
        <html>
        <head>
            <title>Test Page</title>
            <style>
                body { font-family: Arial; margin: 20px; }
                .container { background: #f0f0f0; padding: 20px; }
                h1 { color: #333; }
            </style>
        </head>
        <body>
            <div class="container">
                <h1>Hello World</h1>
                <p>This is a test page for integration testing.</p>
                <button onclick="alert('Clicked!')">Click Me</button>
            </div>
        </body>
        </html>
    "#;
    
    engine.load_html(html).await.unwrap();
    
    let render_result = engine.render_frame().await;
    assert!(render_result.is_ok());
    
    let document = engine.get_document();
    assert!(document.query_selector("h1").is_some());
    assert!(document.query_selector(".container").is_some());
}

#[tokio::test]
async fn test_javascript_execution() {
    let config = Config::default();
    let mut engine = BrowserEngine::new(config, None).unwrap();
    
    engine.load_html(r#"
        <html>
        <body>
            <div id="target">Original</div>
            <script>
                document.getElementById('target').textContent = 'Modified by JS';
                window.testVariable = 'Hello from JavaScript';
            </script>
        </body>
        </html>
    "#).await.unwrap();
    
    let result = engine.execute_script("window.testVariable").await.unwrap();
    assert_eq!(result.as_str(), Some("Hello from JavaScript"));
    
    let target_element = engine.get_document().query_selector("#target").unwrap();
    assert!(target_element.get_text_content().contains("Modified by JS"));
}

#[tokio::test]
async fn test_css_styling_application() {
    let config = Config::default();
    let mut engine = BrowserEngine::new(config, None).unwrap();
    
    engine.load_html(r#"
        <html>
        <head>
            <style>
                .red { color: red; }
                .large { font-size: 24px; }
                #special { background: yellow; }
            </style>
        </head>
        <body>
            <div class="red large" id="special">Styled Text</div>
        </body>
        </html>
    "#).await.unwrap();
    
    let styled_element = engine.get_document().query_selector("#special").unwrap();
    let computed_style = engine.get_computed_style(&styled_element);
    
    assert_eq!(computed_style.get_property("color"), Some("red"));
    assert_eq!(computed_style.get_property("font-size"), Some("24px"));
    assert_eq!(computed_style.get_property("background"), Some("yellow"));
}

#[tokio::test]
async fn test_form_interactions() {
    let config = Config::default();
    let mut engine = BrowserEngine::new(config, None).unwrap();
    
    engine.load_html(r#"
        <html>
        <body>
            <form id="testForm">
                <input type="text" id="username" name="username" value="">
                <input type="password" id="password" name="password" value="">
                <button type="submit">Submit</button>
            </form>
            <div id="result"></div>
            <script>
                document.getElementById('testForm').addEventListener('submit', function(e) {
                    e.preventDefault();
                    document.getElementById('result').textContent = 'Form submitted';
                });
            </script>
        </body>
        </html>
    "#).await.unwrap();
    
    engine.simulate_input("#username", "testuser").await.unwrap();
    engine.simulate_input("#password", "testpass").await.unwrap();
    engine.simulate_click("button[type=submit]").await.unwrap();
    
    let result = engine.get_document().query_selector("#result").unwrap();
    assert!(result.get_text_content().contains("Form submitted"));
}

#[tokio::test]
async fn test_network_requests() {
    let config = Config::default();
    let mut engine = BrowserEngine::new(config, None).unwrap();
    
    engine.load_html(r#"
        <html>
        <body>
            <div id="content">Loading...</div>
            <script>
                fetch('/api/data')
                    .then(response => response.json())
                    .then(data => {
                        document.getElementById('content').textContent = data.message;
                    })
                    .catch(error => {
                        document.getElementById('content').textContent = 'Error: ' + error.message;
                    });
            </script>
        </body>
        </html>
    "#).await.unwrap();
    
    engine.mock_network_response("/api/data", 200, r#"{"message":"Hello from API"}"#).await;
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    let content = engine.get_document().query_selector("#content").unwrap();
    assert!(content.get_text_content().contains("Hello from API"));
}