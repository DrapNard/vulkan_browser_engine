pub mod atlas;

pub use atlas::*;

use crate::renderer::gpu::{Buffer, GpuContext, Texture};
use crate::renderer::pipeline::Pipeline;
use ash::vk;
use rusttype::{Font, Scale, point};
use std::collections::HashMap;
use std::sync::Arc;

pub struct TextRenderer {
    gpu_context: Arc<GpuContext>,
    font_atlas: FontAtlas,
    vertex_buffer: Option<Buffer>,
    fonts: HashMap<String, Font<'static>>,
    default_font_size: f32,
}

#[derive(Debug, Clone)]
pub struct TextVertex {
    pub position: [f32; 2],
    pub tex_coord: [f32; 2],
    pub color: [f32; 4],
}

#[derive(Debug, Clone)]
pub struct GlyphInfo {
    pub character: char,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub advance: f32,
    pub bearing_x: f32,
    pub bearing_y: f32,
}

impl TextRenderer {
    pub async fn new(gpu_context: Arc<GpuContext>) -> Result<Self, TextError> {
        let font_atlas = FontAtlas::new(512, 512)?;
        let mut fonts = HashMap::new();
        
        let default_font_data = include_bytes!("../../../resources/fonts/default.ttf");
        let default_font = Font::try_from_bytes(default_font_data)
            .ok_or_else(|| TextError::FontLoadError("Failed to load default font".to_string()))?;
        
        fonts.insert("default".to_string(), default_font);

        Ok(Self {
            gpu_context,
            font_atlas,
            vertex_buffer: None,
            fonts,
            default_font_size: 16.0,
        })
    }

    pub async fn load_font(&mut self, name: &str, font_data: &[u8]) -> Result<(), TextError> {
        let font = Font::try_from_bytes(font_data)
            .ok_or_else(|| TextError::FontLoadError(format!("Failed to load font: {}", name)))?;
        
        self.fonts.insert(name.to_string(), font);
        Ok(())
    }

    pub async fn render_text(
        &mut self,
        command_buffer: &vk::CommandBuffer,
        text: &str,
        bounds: &crate::core::layout::Rect,
        color: &Option<String>,
        font_family: &Option<String>,
        font_size: f32,
    ) -> Result<(), TextError> {
        let font_name = font_family.as_deref().unwrap_or("default");
        let font = self.fonts.get(font_name)
            .ok_or_else(|| TextError::FontNotFound(font_name.to_string()))?;

        let rgba_color = self.parse_color(color.as_ref().unwrap_or(&"#000000".to_string()));
        let scale = Scale::uniform(font_size);

        let glyphs = self.layout_text(text, font, scale, bounds)?;
        let vertices = self.create_text_vertices(&glyphs, rgba_color)?;

        if vertices.is_empty() {
            return Ok(());
        }

        self.update_vertex_buffer(&vertices).await?;
        self.draw_text_vertices(command_buffer, vertices.len()).await?;

        Ok(())
    }

    fn layout_text(
        &mut self,
        text: &str,
        font: &Font,
        scale: Scale,
        bounds: &crate::core::layout::Rect,
    ) -> Result<Vec<GlyphInfo>, TextError> {
        let mut glyphs = Vec::new();
        let mut x = bounds.x;
        let mut y = bounds.y + font_size_to_baseline(scale.y);

        let v_metrics = font.v_metrics(scale);
        let line_height = v_metrics.ascent - v_metrics.descent + v_metrics.line_gap;

        for character in text.chars() {
            if character == '\n' {
                x = bounds.x;
                y += line_height;
                continue;
            }

            if character == '\r' {
                continue;
            }

            let glyph = font.glyph(character);
            let atlas_coords = self.font_atlas.get_or_cache_glyph(character, &glyph)?;
            
            glyphs.push(GlyphInfo {
                character,
                x: bounding_box.min.x as f32,
                y: bounding_box.min.y as f32,
                width: (bounding_box.max.x - bounding_box.min.x) as f32,
                height: (bounding_box.max.y - bounding_box.min.y) as f32,
                advance: h_metrics.advance_width,
                bearing_x: h_metrics.left_side_bearing,
                bearing_y: v_metrics.ascent,
            });

            x += h_metrics.advance_width;

            if x > bounds.x + bounds.width {
                x = bounds.x;
                y += line_height;
            }
        }

        Ok(glyphs)
    }

    fn create_text_vertices(&self, glyphs: &[GlyphInfo], color: [f32; 4]) -> Result<Vec<TextVertex>, TextError> {
        let mut vertices = Vec::new();

        for glyph in glyphs {
            let atlas_coords = self.font_atlas.get_glyph_coords(glyph.character)
                .ok_or_else(|| TextError::GlyphNotFound(glyph.character))?;

            let quad_vertices = [
                TextVertex {
                    position: [glyph.x, glyph.y],
                    tex_coord: [atlas_coords.u_min, atlas_coords.v_min],
                    color,
                },
                TextVertex {
                    position: [glyph.x + glyph.width, glyph.y],
                    tex_coord: [atlas_coords.u_max, atlas_coords.v_min],
                    color,
                },
                TextVertex {
                    position: [glyph.x + glyph.width, glyph.y + glyph.height],
                    tex_coord: [atlas_coords.u_max, atlas_coords.v_max],
                    color,
                },
                TextVertex {
                    position: [glyph.x, glyph.y + glyph.height],
                    tex_coord: [atlas_coords.u_min, atlas_coords.v_max],
                    color,
                },
            ];

            vertices.extend_from_slice(&quad_vertices);
        }

        Ok(vertices)
    }

    async fn update_vertex_buffer(&mut self, vertices: &[TextVertex]) -> Result<(), TextError> {
        let buffer_size = (vertices.len() * std::mem::size_of::<TextVertex>()) as u64;

        if self.vertex_buffer.is_none() || self.vertex_buffer.as_ref().unwrap().size() < buffer_size {
            self.vertex_buffer = Some(
                self.gpu_context.create_buffer(
                    buffer_size,
                    vk::BufferUsageFlags::VERTEX_BUFFER,
                    gpu_allocator::MemoryLocation::CpuToGpu,
                )?
            );
        }

        if let Some(ref mut buffer) = self.vertex_buffer {
            buffer.write_data(vertices)?;
        }

        Ok(())
    }

    async fn draw_text_vertices(&self, command_buffer: &vk::CommandBuffer, vertex_count: usize) -> Result<(), TextError> {
        if let Some(ref vertex_buffer) = self.vertex_buffer {
            let vertex_buffers = [vertex_buffer.get_buffer()];
            let offsets = [0];

            unsafe {
                self.gpu_context.get_device().cmd_bind_vertex_buffers(
                    *command_buffer,
                    0,
                    &vertex_buffers,
                    &offsets,
                );

                self.gpu_context.get_device().cmd_draw(
                    *command_buffer,
                    vertex_count as u32,
                    1,
                    0,
                    0,
                );
            }
        }

        Ok(())
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

    pub async fn measure_text(
        &self,
        text: &str,
        font_family: &Option<String>,
        font_size: f32,
    ) -> Result<(f32, f32), TextError> {
        let font_name = font_family.as_deref().unwrap_or("default");
        let font = self.fonts.get(font_name)
            .ok_or_else(|| TextError::FontNotFound(font_name.to_string()))?;

        let scale = Scale::uniform(font_size);
        let v_metrics = font.v_metrics(scale);

        let mut width = 0.0f32;
        let mut max_width = 0.0f32;
        let mut lines = 1;

        for character in text.chars() {
            if character == '\n' {
                max_width = max_width.max(width);
                width = 0.0;
                lines += 1;
                continue;
            }

            let glyph = font.glyph(character).scaled(scale);
            let h_metrics = glyph.h_metrics();
            width += h_metrics.advance_width;
        }

        max_width = max_width.max(width);
        let height = (v_metrics.ascent - v_metrics.descent) * lines as f32;

        Ok((max_width, height))
    }

    pub fn get_font_atlas_texture(&self) -> &Texture {
        self.font_atlas.get_texture()
    }

    pub async fn regenerate_atlas(&mut self) -> Result<(), TextError> {
        self.font_atlas.clear();
        Ok(())
    }
}

fn font_size_to_baseline(font_size: f32) -> f32 {
    font_size * 0.8
}

#[derive(Debug, thiserror::Error)]
pub enum TextError {
    #[error("Font load error: {0}")]
    FontLoadError(String),
    #[error("Font not found: {0}")]
    FontNotFound(String),
    #[error("Glyph not found: {0}")]
    GlyphNotFound(char),
    #[error("Atlas error: {0}")]
    AtlasError(#[from] AtlasError),
    #[error("GPU error: {0}")]
    GpuError(#[from] crate::renderer::gpu::GpuError),
    #[error("Render error: {0}")]
    RenderError(String),
}

impl Default for TextRenderer {
    fn default() -> Self {
        panic!("TextRenderer requires GPU context to initialize");
    }
}