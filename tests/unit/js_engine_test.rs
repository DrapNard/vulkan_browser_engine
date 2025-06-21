use vulkan_renderer::js_engine::*;
use serde_json::{Number, Value};
use tokio_test;

#[tokio::test]
async fn test_js_engine_creation() {
    let engine = JsEngine::new().await;
    assert!(engine.is_ok());
}

#[tokio::test]
async fn test_simple_arithmetic() {
    let mut engine = JsEngine::new().await.unwrap();
    
    let result = engine.execute("2 + 2").await.unwrap();
    assert_eq!(result, Value::Number(Number::from(4)));
    
    let result = engine.execute("10 * 5").await.unwrap();
    assert_eq!(result, Value::Number(Number::from(50)));
    
    let result = engine.execute("15 / 3").await.unwrap();
    assert_eq!(result, Value::Number(Number::from(5)));
}

#[tokio::test]
async fn test_string_operations() {
    let mut engine = JsEngine::new().await.unwrap();
    
    let result = engine.execute("'Hello' + ' ' + 'World'").await.unwrap();
    assert_eq!(result, Value::String("Hello World".to_string()));
    
    let result = engine.execute("'test'.toUpperCase()").await.unwrap();
    assert_eq!(result, Value::String("TEST".to_string()));
}

#[tokio::test]
async fn test_variables_and_functions() {
    let mut engine = JsEngine::new().await.unwrap();
    
    engine.execute("let x = 10; let y = 20;").await.unwrap();
    let result = engine.execute("x + y").await.unwrap();
    assert_eq!(result, Value::Number(Number::from(30)));
    
    engine.execute("function add(a, b) { return a + b; }").await.unwrap();
    let result = engine.execute("add(5, 7)").await.unwrap();
    assert_eq!(result, Value::Number(Number::from(12)));
}

#[tokio::test]
async fn test_objects_and_arrays() {
    let mut engine = JsEngine::new().await.unwrap();
    
    let result = engine.execute("let obj = {name: 'test', value: 42}; obj.name").await.unwrap();
    assert_eq!(result, Value::String("test".to_string()));
    
    let result = engine.execute("let arr = [1, 2, 3]; arr.length").await.unwrap();
    assert_eq!(result, Value::Number(Number::from(3)));
    
    let result = engine.execute("arr[1]").await.unwrap();
    assert_eq!(result, Value::Number(Number::from(2)));
}

#[tokio::test]
async fn test_console_log() {
    let mut engine = JsEngine::new().await.unwrap();
    
    let result = engine.execute("console.log('Hello from JS'); 'done'").await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), Value::String("done".to_string()));
}

#[tokio::test]
async fn test_error_handling() {
    let mut engine = JsEngine::new().await.unwrap();
    
    let result = engine.execute("throw new Error('Test error')").await;
    assert!(result.is_err());
    
    let result = engine.execute("undefined_variable").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_loops_and_conditionals() {
    let mut engine = JsEngine::new().await.unwrap();
    
    let result = engine.execute(r#"
        let sum = 0;
        for (let i = 1; i <= 10; i++) {
            sum += i;
        }
        sum
    "#).await.unwrap();
    assert_eq!(result, Value::Number(Number::from(55)));
    
    let result = engine.execute(r#"
        let x = 10;
        if (x > 5) {
            'greater'
        } else {
            'lesser'
        }
    "#).await.unwrap();
    assert_eq!(result, Value::String("greater".to_string()));
}

#[tokio::test]
async fn test_json_operations() {
    let mut engine = JsEngine::new().await.unwrap();
    
    let result = engine.execute(r#"
        let obj = {name: "John", age: 30};
        JSON.stringify(obj)
    "#).await.unwrap();
    assert!(result.as_str().unwrap().contains("John"));
    
    let result = engine.execute(r#"
        let jsonStr = '{"test": true}';
        JSON.parse(jsonStr).test
    "#).await.unwrap();
    assert_eq!(result, Value::Bool(true));
}

#[tokio::test]
async fn test_async_operations() {
    let mut engine = JsEngine::new().await.unwrap();
    
    let result = engine.execute(r#"
        new Promise((resolve) => {
            setTimeout(() => resolve('async result'), 10);
        })
    "#).await;
    
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_memory_isolation() {
    let mut engine1 = JsEngine::new().await.unwrap();
    let mut engine2 = JsEngine::new().await.unwrap();
    
    engine1.execute("let testVar = 'engine1'").await.unwrap();
    engine2.execute("let testVar = 'engine2'").await.unwrap();
    
    let result1 = engine1.execute("testVar").await.unwrap();
    let result2 = engine2.execute("testVar").await.unwrap();
    
    assert_eq!(result1, Value::String("engine1".to_string()));
    assert_eq!(result2, Value::String("engine2".to_string()));
}
