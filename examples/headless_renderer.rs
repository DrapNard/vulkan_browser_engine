use vulkan_renderer::{Config, HeadlessRenderer};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config {
        headless: true,
        width: 1920,
        height: 1080,
        ..Default::default()
    };
    
    let mut renderer = HeadlessRenderer::new(config)?;
    
    let html = r#"
        <!DOCTYPE html>
        <html>
        <head>
            <style>
                body { font-family: Arial; background: linear-gradient(45deg, #ff6b6b, #4ecdc4); }
                .container { max-width: 800px; margin: 50px auto; padding: 20px; background: white; border-radius: 10px; }
            </style>
        </head>
        <body>
            <div class="container">
                <h1>Headless Rendering Test</h1>
                <p>This page is rendered without a window!</p>
            </div>
        </body>
        </html>
    "#;
    
    renderer.load_html(html)?;
    let image_data = renderer.render_to_buffer()?;
    
    image::save_buffer("output.png", &image_data, 1920, 1080, image::ColorType::Rgba8)?;
    println!("Rendered to output.png");
    
    Ok(())
}