pub mod gpu;
pub mod image;
pub mod pipeline;
pub mod text;
pub mod vulkan;

use crate::core::dom::Document;
use crate::core::layout::LayoutTree;
use std::sync::Arc;
use vulkan::VulkanContext;

pub struct VulkanRenderer {
    context: Arc<VulkanContext>,
    pipeline_cache: pipeline::PipelineCache,
    text_renderer: text::TextRenderer,
    image_loader: image::ImageLoader,
}

impl VulkanRenderer {
    pub async fn new() -> Result<Self, RenderError> {
        let context = Arc::new(VulkanContext::new().await?);
        let pipeline_cache = pipeline::PipelineCache::new(context.clone()).await?;
        let text_renderer = text::TextRenderer::new(context.clone()).await?;
        let image_loader = image::ImageLoader::new();

        Ok(Self {
            context,
            pipeline_cache,
            text_renderer,
            image_loader,
        })
    }

    pub async fn render(&mut self, document: &Document, layout_tree: &LayoutTree) -> Result<(), RenderError> {
        let command_buffer = self.context.begin_frame().await?;
        
        self.render_background(&command_buffer).await?;
        self.render_elements(&command_buffer, document, layout_tree).await?;
        self.render_text(&command_buffer, layout_tree).await?;
        
        self.context.end_frame(command_buffer).await?;
        Ok(())
    }

    async fn render_background(&self, command_buffer: &vulkan::CommandBuffer) -> Result<(), RenderError> {
        // Clear background with default color
        command_buffer.clear_color([0.0, 0.0, 0.0, 1.0]).await?;
        Ok(())
    }

    async fn render_elements(&self, command_buffer: &vulkan::CommandBuffer, document: &Document, layout_tree: &LayoutTree) -> Result<(), RenderError> {
        for node in layout_tree.get_render_nodes() {
            match node.element_type {
                crate::core::layout::ElementType::Block => {
                    self.render_block_element(command_buffer, node).await?;
                }
                crate::core::layout::ElementType::Inline => {
                    self.render_inline_element(command_buffer, node).await?;
                }
                crate::core::layout::ElementType::Image => {
                    self.render_image_element(command_buffer, node).await?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn render_block_element(&self, command_buffer: &vulkan::CommandBuffer, node: &crate::core::layout::LayoutNode) -> Result<(), RenderError> {
        let pipeline = self.pipeline_cache.get_rect_pipeline().await?;
        
        let vertices = self.create_rect_vertices(&node.bounds, &node.style.background_color);
        
        command_buffer.bind_pipeline(&pipeline).await?;
        command_buffer.draw_vertices(&vertices).await?;
        
        Ok(())
    }

    async fn render_inline_element(&self, command_buffer: &vulkan::CommandBuffer, node: &crate::core::layout::LayoutNode) -> Result<(), RenderError> {
        // Inline elements are typically rendered as text
        Ok(())
    }

    async fn render_image_element(&self, command_buffer: &vulkan::CommandBuffer, node: &crate::core::layout::LayoutNode) -> Result<(), RenderError> {
        if let Some(image_url) = &node.image_url {
            let texture = self.image_loader.load_image(image_url).await?;
            let pipeline = self.pipeline_cache.get_image_pipeline().await?;
            
            let vertices = self.create_image_vertices(&node.bounds);
            
            command_buffer.bind_pipeline(&pipeline).await?;
            command_buffer.bind_texture(&texture).await?;
            command_buffer.draw_vertices(&vertices).await?;
        }
        Ok(())
    }

    async fn render_text(&self, command_buffer: &vulkan::CommandBuffer, layout_tree: &LayoutTree) -> Result<(), RenderError> {
        for node in layout_tree.get_text_nodes() {
            if let Some(text_content) = &node.text_content {
                self.text_renderer.render_text(
                    command_buffer,
                    text_content,
                    &node.bounds,
                    &node.style.color,
                    &node.style.font_family,
                    node.style.font_size,
                ).await?;
            }
        }
        Ok(())
    }

    fn create_rect_vertices(&self, bounds: &crate::core::layout::Rect, color: &Option<String>) -> Vec<Vertex> {
        let rgba = self.parse_color(color.as_ref().unwrap_or(&"#000000".to_string()));
        
        vec![
            Vertex { position: [bounds.x, bounds.y, 0.0], tex_coord: [0.0, 0.0], color: rgba },
            Vertex { position: [bounds.x + bounds.width, bounds.y, 0.0], tex_coord: [1.0, 0.0], color: rgba },
            Vertex { position: [bounds.x + bounds.width, bounds.y + bounds.height, 0.0], tex_coord: [1.0, 1.0], color: rgba },
            Vertex { position: [bounds.x, bounds.y + bounds.height, 0.0], tex_coord: [0.0, 1.0], color: rgba },
        ]
    }

    fn create_image_vertices(&self, bounds: &crate::core::layout::Rect) -> Vec<Vertex> {
        let white = [1.0, 1.0, 1.0, 1.0];
        
        vec![
            Vertex { position: [bounds.x, bounds.y, 0.0], tex_coord: [0.0, 0.0], color: white },
            Vertex { position: [bounds.x + bounds.width, bounds.y, 0.0], tex_coord: [1.0, 0.0], color: white },
            Vertex { position: [bounds.x + bounds.width, bounds.y + bounds.height, 0.0], tex_coord: [1.0, 1.0], color: white },
            Vertex { position: [bounds.x, bounds.y + bounds.height, 0.0], tex_coord: [0.0, 1.0], color: white },
        ]
    }

    fn parse_color(&self, color_str: &str) -> [f32; 4] {
        if color_str.starts_with('#') && color_str.len() == 7 {
            let r = u8::from_str_radix(&color_str[1..3], 16).unwrap_or(0) as f32 / 255.0;
            let g = u8::from_str_radix(&color_str[3..5], 16).unwrap_or(0) as f32 / 255.0;
            let b = u8::from_str_radix(&color_str[5..7], 16).unwrap_or(0) as f32 / 255.0;
            [r, g, b, 1.0]
        } else {
            [0.0, 0.0, 0.0, 1.0]
        }
    }

    pub async fn resize(&mut self, width: u32, height: u32) -> Result<(), RenderError> {
        self.context.resize(width, height).await?;
        Ok(())
    }

    pub async fn present(&mut self) -> Result<(), RenderError> {
        self.context.present().await?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Vertex {
    pub position: [f32; 3],
    pub tex_coord: [f32; 2],
    pub color: [f32; 4],
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("Vulkan error: {0}")]
    VulkanError(#[from] vulkan::VulkanError),
    #[error("Pipeline error: {0}")]
    PipelineError(#[from] pipeline::PipelineError),
    #[error("Text render error: {0}")]
    TextRenderError(#[from] text::TextError),
    #[error("Image load error: {0}")]
    ImageLoadError(#[from] image::ImageError),
    #[error("Resource error: {0}")]
    ResourceError(String),
}

impl Default for VulkanRenderer {
    fn default() -> Self {
        futures::executor::block_on(async { Self::new().await.unwrap() })
    }
}