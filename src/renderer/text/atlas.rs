use crate::renderer::gpu::Texture;
use rusttype::{point, Glyph, Scale};
use std::collections::HashMap;

pub struct FontAtlas {
    width: u32,
    height: u32,
    texture: Option<Texture>,
    glyph_cache: HashMap<char, GlyphCoords>,
    current_x: u32,
    current_y: u32,
    row_height: u32,
    data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct GlyphCoords {
    pub u_min: f32,
    pub v_min: f32,
    pub u_max: f32,
    pub v_max: f32,
    pub width: u32,
    pub height: u32,
}

impl FontAtlas {
    pub fn new(width: u32, height: u32) -> Result<Self, AtlasError> {
        let data = vec![0u8; (width * height) as usize];

        Ok(Self {
            width,
            height,
            texture: None,
            glyph_cache: HashMap::new(),
            current_x: 0,
            current_y: 0,
            row_height: 0,
            data,
        })
    }

    pub fn get_or_cache_glyph(
        &mut self,
        character: char,
        glyph: &Glyph<'_>,
        scale: Scale,
    ) -> Result<GlyphCoords, AtlasError> {
        if let Some(coords) = self.glyph_cache.get(&character) {
            return Ok(coords.clone());
        }

        // Scale the glyph first, then position it
        let scaled_glyph = glyph.clone().scaled(scale);
        let positioned_glyph = scaled_glyph.positioned(point(0.0, 0.0));

        if let Some(bounding_box) = positioned_glyph.pixel_bounding_box() {
            let glyph_width = (bounding_box.max.x - bounding_box.min.x) as u32;
            let glyph_height = (bounding_box.max.y - bounding_box.min.y) as u32;

            if glyph_width == 0 || glyph_height == 0 {
                let coords = GlyphCoords {
                    u_min: 0.0,
                    v_min: 0.0,
                    u_max: 0.0,
                    v_max: 0.0,
                    width: 0,
                    height: 0,
                };
                self.glyph_cache.insert(character, coords.clone());
                return Ok(coords);
            }

            let atlas_pos = self.allocate_space(glyph_width, glyph_height)?;
            self.rasterize_glyph(&positioned_glyph, atlas_pos.0, atlas_pos.1)?;

            let coords = GlyphCoords {
                u_min: atlas_pos.0 as f32 / self.width as f32,
                v_min: atlas_pos.1 as f32 / self.height as f32,
                u_max: (atlas_pos.0 + glyph_width) as f32 / self.width as f32,
                v_max: (atlas_pos.1 + glyph_height) as f32 / self.height as f32,
                width: glyph_width,
                height: glyph_height,
            };

            self.glyph_cache.insert(character, coords.clone());
            Ok(coords)
        } else {
            Err(AtlasError::GlyphRasterizationFailed(character))
        }
    }

    fn allocate_space(&mut self, width: u32, height: u32) -> Result<(u32, u32), AtlasError> {
        if self.current_x + width > self.width {
            self.current_x = 0;
            self.current_y += self.row_height;
            self.row_height = 0;
        }

        if self.current_y + height > self.height {
            return Err(AtlasError::AtlasFull);
        }

        let position = (self.current_x, self.current_y);
        self.current_x += width;
        self.row_height = self.row_height.max(height);

        Ok(position)
    }

    fn rasterize_glyph(
        &mut self,
        glyph: &rusttype::PositionedGlyph,
        atlas_x: u32,
        atlas_y: u32,
    ) -> Result<(), AtlasError> {
        glyph.draw(|x, y, v| {
            let atlas_pixel_x = atlas_x + x;
            let atlas_pixel_y = atlas_y + y;

            if atlas_pixel_x < self.width && atlas_pixel_y < self.height {
                let index = (atlas_pixel_y * self.width + atlas_pixel_x) as usize;
                if index < self.data.len() {
                    self.data[index] = (v * 255.0) as u8;
                }
            }
        });

        Ok(())
    }

    pub fn get_glyph_coords(&self, character: char) -> Option<&GlyphCoords> {
        self.glyph_cache.get(&character)
    }

    pub fn get_texture(&self) -> &Texture {
        self.texture.as_ref().expect("Texture not created")
    }

    pub fn update_texture(
        &mut self,
        _gpu_context: &crate::renderer::gpu::GpuContext,
    ) -> Result<(), AtlasError> {
        // This would update the GPU texture with the current atlas data
        // Implementation would depend on the specific GPU context setup
        Ok(())
    }

    pub fn clear(&mut self) {
        self.glyph_cache.clear();
        self.current_x = 0;
        self.current_y = 0;
        self.row_height = 0;
        self.data.fill(0);
    }

    pub fn get_atlas_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn get_usage_stats(&self) -> AtlasUsageStats {
        let used_pixels = self.current_y * self.width + self.current_x;
        let total_pixels = self.width * self.height;
        let usage_percentage = (used_pixels as f32 / total_pixels as f32) * 100.0;

        AtlasUsageStats {
            used_pixels,
            total_pixels,
            usage_percentage,
            cached_glyphs: self.glyph_cache.len(),
        }
    }

    pub fn needs_rebuild(&self) -> bool {
        let stats = self.get_usage_stats();
        stats.usage_percentage > 90.0
    }

    pub fn rebuild_with_size(&mut self, new_width: u32, new_height: u32) -> Result<(), AtlasError> {
        self.width = new_width;
        self.height = new_height;
        self.data = vec![0u8; (new_width * new_height) as usize];
        self.clear();
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct AtlasUsageStats {
    pub used_pixels: u32,
    pub total_pixels: u32,
    pub usage_percentage: f32,
    pub cached_glyphs: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum AtlasError {
    #[error("Atlas is full")]
    AtlasFull,
    #[error("Glyph rasterization failed for character: {0}")]
    GlyphRasterizationFailed(char),
    #[error("Texture creation failed")]
    TextureCreationFailed,
    #[error("Invalid atlas dimensions")]
    InvalidDimensions,
}

impl Default for FontAtlas {
    fn default() -> Self {
        Self::new(512, 512).expect("Failed to create default font atlas")
    }
}
