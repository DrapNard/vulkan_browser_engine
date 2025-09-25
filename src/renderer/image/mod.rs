pub mod loader;

pub use loader::*;

use crate::renderer::gpu::Texture;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ImageManager {
    loader: ImageLoader,
    texture_cache: Arc<RwLock<HashMap<String, Arc<Texture>>>>,
    max_cache_size: usize,
}

impl ImageManager {
    pub fn new() -> Self {
        Self {
            loader: ImageLoader::new(),
            texture_cache: Arc::new(RwLock::new(HashMap::new())),
            max_cache_size: 1000,
        }
    }

    pub async fn load_texture(
        &self,
        url: &str,
        gpu_context: &crate::renderer::gpu::GpuContext,
    ) -> Result<Arc<Texture>, ImageError> {
        {
            let cache = self.texture_cache.read().await;
            if let Some(texture) = cache.get(url) {
                return Ok(texture.clone());
            }
        }

        let texture = self.loader.load_image(url, gpu_context).await?;
        let texture_arc = Arc::new(texture);

        {
            let mut cache = self.texture_cache.write().await;
            if cache.len() >= self.max_cache_size {
                self.evict_least_used(&mut cache).await;
            }
            cache.insert(url.to_string(), texture_arc.clone());
        }

        Ok(texture_arc)
    }

    async fn evict_least_used(&self, cache: &mut HashMap<String, Arc<Texture>>) {
        if let Some(key) = cache.keys().next().cloned() {
            cache.remove(&key);
        }
    }

    pub async fn preload_images(
        &self,
        urls: &[String],
        gpu_context: &crate::renderer::gpu::GpuContext,
    ) -> Result<(), ImageError> {
        for url in urls {
            self.load_texture(url, gpu_context).await?;
        }
        Ok(())
    }

    pub async fn clear_cache(&self) {
        let mut cache = self.texture_cache.write().await;
        cache.clear();
    }

    pub async fn get_cache_stats(&self) -> ImageCacheStats {
        let cache = self.texture_cache.read().await;
        ImageCacheStats {
            cached_images: cache.len(),
            max_cache_size: self.max_cache_size,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImageCacheStats {
    pub cached_images: usize,
    pub max_cache_size: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("Load error: {0}")]
    LoadError(String),
    #[error("Decode error: {0}")]
    DecodeError(String),
    #[error("GPU error: {0}")]
    GpuError(#[from] crate::renderer::gpu::GpuError),
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),
}

impl Default for ImageManager {
    fn default() -> Self {
        Self::new()
    }
}
