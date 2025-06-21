use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};

pub mod dom_bench;
pub mod js_bench; 
pub mod render_bench;

criterion_group!(benches, dom_bench::bench_dom, js_bench::bench_js, render_bench::bench_render);
criterion_main!(benches);

tests/benchmark/dom_bench.rs
use criterion::{Criterion, BenchmarkId};
use vulkan_renderer::core::dom::*;

pub fn bench_dom(c: &mut Criterion) {
    c.bench_function("dom_creation", |b| {
        b.iter(|| {
            let _doc = Document::new();
        })
    });

    c.bench_function("element_insertion", |b| {
        b.iter(|| {
            let mut doc = Document::new();
            for i in 0..1000 {
                let element = Element::new(format!("div{}", i));
                doc.insert_element(element);
            }
        })
    });

    let html_sizes = vec![1000, 5000, 10000];
    for size in html_sizes {
        c.bench_with_input(
            BenchmarkId::new("html_parsing", size),
            &size,
            |b, &size| {
                let html = format!("<div>{}</div>", "a".repeat(size));
                b.iter(|| {
                    let _doc = Document::parse(&html).unwrap();
                })
            }
        );
    }
}