use super::ImageError;
use crate::renderer::gpu::{GpuContext, Texture};
use ash::vk;
use image::{DynamicImage, ImageFormat};
use std::io::Cursor;
use base64::engine::Engine; // Import the trait for decode method

pub struct ImageLoader {
    supported_formats: Vec<ImageFormat>,
}

impl ImageLoader {
    pub fn new() -> Self {
        Self {
            supported_formats: vec![
                ImageFormat::Png,
                ImageFormat::Jpeg,
                ImageFormat::WebP,
                ImageFormat::Gif,
                ImageFormat::Bmp,
                ImageFormat::Tiff,
            ],
        }
    }

    pub async fn load_image(&self, url: &str, gpu_context: &GpuContext) -> Result<Texture, ImageError> {
        let image_data = self.fetch_image_data(url).await?;
        let image = self.decode_image(&image_data)?;
        self.create_texture_with_context(image, gpu_context)
    }

    pub async fn load_image_from_bytes(&self, data: &[u8], gpu_context: &GpuContext) -> Result<Texture, ImageError> {
        let image = self.decode_image(data)?;
        self.create_texture_with_context(image, gpu_context)
    }

    pub async fn load_image_data(&self, url: &str) -> Result<DynamicImage, ImageError> {
        let image_data = self.fetch_image_data(url).await?;
        self.decode_image(&image_data)
    }

    pub fn load_image_data_from_bytes(&self, data: &[u8]) -> Result<DynamicImage, ImageError> {
        self.decode_image(data)
    }

    async fn fetch_image_data(&self, url: &str) -> Result<Vec<u8>, ImageError> {
        if url.starts_with("data:") {
            self.parse_data_url(url)
        } else if url.starts_with("http://") || url.starts_with("https://") {
            self.fetch_remote_image(url).await
        } else {
            self.load_local_image(url).await
        }
    }

    fn parse_data_url(&self, url: &str) -> Result<Vec<u8>, ImageError> {
        if let Some(comma_pos) = url.find(',') {
            let data_part = &url[comma_pos + 1..];
            base64::engine::general_purpose::STANDARD
                .decode(data_part)
                .map_err(|e| ImageError::DecodeError(e.to_string()))
        } else {
            Err(ImageError::DecodeError("Invalid data URL".to_string()))
        }
    }

    async fn fetch_remote_image(&self, url: &str) -> Result<Vec<u8>, ImageError> {
        let response = reqwest::get(url)
            .await
            .map_err(|e| ImageError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(ImageError::NetworkError(format!(
                "HTTP error: {}",
                response.status()
            )));
        }

        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|e| ImageError::NetworkError(e.to_string()))
    }

    async fn load_local_image(&self, path: &str) -> Result<Vec<u8>, ImageError> {
        tokio::fs::read(path)
            .await
            .map_err(|e| ImageError::LoadError(e.to_string()))
    }

    fn decode_image(&self, data: &[u8]) -> Result<DynamicImage, ImageError> {
        let format = image::guess_format(data)
            .map_err(|e| ImageError::DecodeError(e.to_string()))?;

        if !self.supported_formats.contains(&format) {
            return Err(ImageError::UnsupportedFormat(format!("{:?}", format)));
        }

        let cursor = Cursor::new(data);
        image::load(cursor, format)
            .map_err(|e| ImageError::DecodeError(e.to_string()))
    }

    pub fn create_texture_with_context(
        &self,
        image: DynamicImage,
        gpu_context: &GpuContext,
    ) -> Result<Texture, ImageError> {
        let rgba_image = image.to_rgba8();
        let (width, height) = rgba_image.dimensions();
        let image_data = rgba_image.into_raw();

        let staging_buffer = gpu_context.create_buffer(
            image_data.len() as u64,
            vk::BufferUsageFlags::TRANSFER_SRC,
            gpu_allocator::MemoryLocation::CpuToGpu,
        )?;

        let mut staging_buffer = staging_buffer;
        staging_buffer.write_data(&image_data)?;

        let texture = gpu_context.create_texture(
            width,
            height,
            vk::Format::R8G8B8A8_SRGB,
            vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
        )?;

        let cmd = gpu_context.allocate_command_buffer()?;

        unsafe {
            gpu_context.get_device().begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::builder()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            ).map_err(crate::renderer::gpu::GpuError::VulkanError)?;
        }

        texture.transition_layout(
            cmd,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        )?;

        texture.copy_from_buffer(cmd, staging_buffer.get_buffer())?;

        texture.transition_layout(
            cmd,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        )?;

        texture.generate_mipmaps(cmd)?;

        unsafe {
            gpu_context.get_device().end_command_buffer(cmd)
                .map_err(crate::renderer::gpu::GpuError::VulkanError)?;
        }

        gpu_context.submit_command_buffer(cmd, None)?;
        gpu_context.wait_idle()?;

        Ok(texture)
    }

    pub fn create_placeholder_texture(&self, width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let pixel_count = (width * height) as usize;
        let mut data = Vec::with_capacity(pixel_count * 4);
        
        for _ in 0..pixel_count {
            data.extend_from_slice(&color);
        }
        
        data
    }

    pub fn create_placeholder_texture_with_context(&self, width: u32, height: u32, color: [u8; 4], gpu_context: &GpuContext) -> Result<Texture, ImageError> {
        let data = self.create_placeholder_texture(width, height, color);
        let image = image::RgbaImage::from_raw(width, height, data)
            .ok_or_else(|| ImageError::LoadError("Failed to create placeholder image".to_string()))?;
        let dynamic_image = DynamicImage::ImageRgba8(image);
        self.create_texture_with_context(dynamic_image, gpu_context)
    }

    pub async fn create_texture_atlas(&self, images: &[DynamicImage]) -> Result<DynamicImage, ImageError> {
        if images.is_empty() {
            return Err(ImageError::LoadError("No images provided".to_string()));
        }

        let total_width: u32 = images.iter().map(|img| img.width()).sum();
        let max_height: u32 = images.iter().map(|img| img.height()).max().unwrap_or(0);

        let mut atlas = image::RgbaImage::new(total_width, max_height);
        let mut x_offset = 0;

        for img in images {
            let rgba_img = img.to_rgba8();
            image::imageops::overlay(&mut atlas, &rgba_img, x_offset as i64, 0);
            x_offset += img.width();
        }

        Ok(DynamicImage::ImageRgba8(atlas))
    }

    pub async fn create_texture_atlas_with_context(&self, images: &[DynamicImage], gpu_context: &GpuContext) -> Result<Texture, ImageError> {
        let atlas_image = self.create_texture_atlas(images).await?;
        self.create_texture_with_context(atlas_image, gpu_context)
    }
}

impl Default for ImageLoader {
    fn default() -> Self {
        Self::new()
    }
}