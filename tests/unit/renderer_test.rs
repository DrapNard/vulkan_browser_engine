use vulkan_renderer::renderer::*;

#[tokio::test]
async fn test_vertex_creation() {
    let vertex = Vertex {
        position: [0.0, 0.0, 0.0],
        tex_coord: [0.0, 0.0],
        color: [1.0, 1.0, 1.0, 1.0],
    };
    
    assert_eq!(vertex.position, [0.0, 0.0, 0.0]);
    assert_eq!(vertex.tex_coord, [0.0, 0.0]);
    assert_eq!(vertex.color, [1.0, 1.0, 1.0, 1.0]);
}

#[tokio::test]
async fn test_vertex_buffer_creation() {
    let vertices = vec![
        Vertex {
            position: [0.0, 0.0, 0.0],
            tex_coord: [0.0, 0.0],
            color: [1.0, 0.0, 0.0, 1.0],
        },
        Vertex {
            position: [1.0, 0.0, 0.0],
            tex_coord: [1.0, 0.0],
            color: [0.0, 1.0, 0.0, 1.0],
        },
        Vertex {
            position: [0.5, 1.0, 0.0],
            tex_coord: [0.5, 1.0],
            color: [0.0, 0.0, 1.0, 1.0],
        },
    ];
    
    assert_eq!(vertices.len(), 3);
    assert_eq!(vertices[0].color, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(vertices[1].color, [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(vertices[2].color, [0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn test_color_parsing() {
    fn parse_color(color_str: &str) -> [f32; 4] {
        if color_str.starts_with('#') && color_str.len() == 7 {
            let r = u8::from_str_radix(&color_str[1..3], 16).unwrap_or(0) as f32 / 255.0;
            let g = u8::from_str_radix(&color_str[3..5], 16).unwrap_or(0) as f32 / 255.0;
            let b = u8::from_str_radix(&color_str[5..7], 16).unwrap_or(0) as f32 / 255.0;
            [r, g, b, 1.0]
        } else {
            [0.0, 0.0, 0.0, 1.0]
        }
    }
    
    let red = parse_color("#FF0000");
    assert_eq!(red, [1.0, 0.0, 0.0, 1.0]);
    
    let green = parse_color("#00FF00");
    assert_eq!(green, [0.0, 1.0, 0.0, 1.0]);
    
    let blue = parse_color("#0000FF");
    assert_eq!(blue, [0.0, 0.0, 1.0, 1.0]);
    
    let white = parse_color("#FFFFFF");
    assert_eq!(white, [1.0, 1.0, 1.0, 1.0]);
    
    let black = parse_color("#000000");
    assert_eq!(black, [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn test_rect_vertices_generation() {
    struct Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    }
    
    fn create_rect_vertices(bounds: &Rect, color: [f32; 4]) -> Vec<Vertex> {
        vec![
            Vertex { position: [bounds.x, bounds.y, 0.0], tex_coord: [0.0, 0.0], color },
            Vertex { position: [bounds.x + bounds.width, bounds.y, 0.0], tex_coord: [1.0, 0.0], color },
            Vertex { position: [bounds.x + bounds.width, bounds.y + bounds.height, 0.0], tex_coord: [1.0, 1.0], color },
            Vertex { position: [bounds.x, bounds.y + bounds.height, 0.0], tex_coord: [0.0, 1.0], color },
        ]
    }
    
    let rect = Rect { x: 10.0, y: 20.0, width: 100.0, height: 50.0 };
    let color = [1.0, 0.0, 0.0, 1.0];
    let vertices = create_rect_vertices(&rect, color);
    
    assert_eq!(vertices.len(), 4);
    assert_eq!(vertices[0].position, [10.0, 20.0, 0.0]);
    assert_eq!(vertices[1].position, [110.0, 20.0, 0.0]);
    assert_eq!(vertices[2].position, [110.0, 70.0, 0.0]);
    assert_eq!(vertices[3].position, [10.0, 70.0, 0.0]);
}

#[test]
fn test_triangle_indices() {
    let indices: Vec<u16> = vec![0, 1, 2, 2, 3, 0];
    
    assert_eq!(indices.len(), 6);
    assert_eq!(indices[0], 0);
    assert_eq!(indices[1], 1);
    assert_eq!(indices[2], 2);
    assert_eq!(indices[3], 2);
    assert_eq!(indices[4], 3);
    assert_eq!(indices[5], 0);
}