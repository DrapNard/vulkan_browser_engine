use criterion::{Criterion, BenchmarkId};
use vulkan_renderer::js_engine::*;

pub fn bench_js(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    
    c.bench_function("js_engine_creation", |b| {
        b.to_async(&rt).iter(|| async {
            let _engine = JsEngine::new().await.unwrap();
        })
    });

    c.bench_function("simple_expression", |b| {
        b.to_async(&rt).iter(|| async {
            let mut engine = JsEngine::new().await.unwrap();
            let _result = engine.execute("2 + 2").await.unwrap();
        })
    });

    let script_sizes = vec![100, 1000, 10000];
    for size in script_sizes {
        c.bench_with_input(
            BenchmarkId::new("script_execution", size),
            &size,
            |b, &size| {
                let script = format!("let sum = 0; for(let i = 0; i < {}; i++) {{ sum += i; }} sum", size);
                b.to_async(&rt).iter(|| async {
                    let mut engine = JsEngine::new().await.unwrap();
                    let _result = engine.execute(&script).await.unwrap();
                })
            }
        );
    }
}