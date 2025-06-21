use criterion::{Criterion, BenchmarkId};

pub fn bench_render(c: &mut Criterion) {
    c.bench_function("vertex_creation", |b| {
        b.iter(|| {
            let _vertices: Vec<vulkan_renderer::renderer::Vertex> = (0..1000)
                .map(|i| vulkan_renderer::renderer::Vertex {
                    position: [i as f32, i as f32, 0.0],
                    tex_coord: [0.0, 0.0],
                    color: [1.0, 1.0, 1.0, 1.0],
                })
                .collect();
        })
    });
}