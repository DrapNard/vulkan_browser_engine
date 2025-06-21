use criterion::{Criterion, BenchmarkId};
use vulkan_renderer::core::dom::*;

pub fn bench_dom(c: &mut Criterion) {
    c.bench_function("dom_creation", |b| {
        b.iter(|| {
            let _doc = Document::new();
        })
    });

    c.bench_function("single_element_insertion", |b| {
        b.iter(|| {
            let mut doc = Document::new();
            let element = Element::new("div".to_string());
            doc.insert_element(element);
        })
    });

    c.bench_function("multiple_element_insertion", |b| {
        b.iter(|| {
            let mut doc = Document::new();
            for i in 0..1000 {
                let element = Element::new(format!("div{}", i));
                doc.insert_element(element);
            }
        })
    });

    let html_sizes = vec![100, 1000, 5000, 10000];
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

    c.bench_function("nested_elements", |b| {
        b.iter(|| {
            let mut doc = Document::new();
            let mut current_parent_id = None;
            
            for i in 0..100 {
                let mut element = Element::new("div".to_string());
                element.set_attribute("id".to_string(), format!("element_{}", i));
                
                if let Some(parent_id) = current_parent_id {
                    if let Some(parent) = doc.get_element_mut(parent_id) {
                        let child_id = doc.insert_element(element);
                        parent.children.push(child_id);
                        current_parent_id = Some(child_id);
                    }
                } else {
                    current_parent_id = Some(doc.insert_element(element));
                }
            }
        })
    });

    c.bench_function("attribute_operations", |b| {
        b.iter(|| {
            let mut element = Element::new("div".to_string());
            for i in 0..50 {
                element.set_attribute(format!("attr_{}", i), format!("value_{}", i));
            }
            
            for i in 0..50 {
                let _ = element.get_attribute(&format!("attr_{}", i));
            }
        })
    });

    c.bench_function("query_selector", |b| {
        let html = (0..1000)
            .map(|i| format!(r#"<div id="element_{}" class="test">Content {}</div>"#, i, i))
            .collect::<Vec<_>>()
            .join("");
        let doc = Document::parse(&html).unwrap();
        
        b.iter(|| {
            let _result = doc.query_selector("#element_500");
            let _results = doc.query_selector_all(".test");
        })
    });
}