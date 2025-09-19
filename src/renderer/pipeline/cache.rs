use super::{Pipeline, PipelineError, PipelineManager};
use crate::renderer::text::TextVertex;
use ash::vk;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::log;

pub struct PipelineCache {
    manager: Arc<RwLock<PipelineManager>>,
    shader_cache: Arc<RwLock<HashMap<String, ShaderData>>>,
    hot_reload_enabled: bool,
}

#[derive(Clone)]
struct ShaderData {
    content: Vec<u8>,
    hash: u64,
    last_modified: std::time::SystemTime,
}

#[derive(Debug, Clone, Copy)]
pub enum PipelineType {
    Rect,
    Image,
    Text,
}

#[derive(Debug, Clone, Copy)]
enum VertexType {
    Standard,
    Text,
}

struct PipelineSpec {
    name: &'static str,
    vertex_shader: &'static str,
    fragment_shader: &'static str,
    vertex_type: VertexType,
}

impl PipelineCache {
    pub async fn new(
        device: Arc<ash::Device>,
        render_pass: vk::RenderPass,
    ) -> Result<Self, PipelineError> {
        let manager = PipelineManager::new(device, render_pass)?;

        Ok(Self {
            manager: Arc::new(RwLock::new(manager)),
            shader_cache: Arc::new(RwLock::new(HashMap::new())),
            hot_reload_enabled: cfg!(debug_assertions),
        })
    }

    pub async fn get_rect_pipeline(&self) -> Result<Pipeline, PipelineError> {
        self.get_or_create_graphics_pipeline(PipelineType::Rect).await
    }

    pub async fn get_image_pipeline(&self) -> Result<Pipeline, PipelineError> {
        self.get_or_create_graphics_pipeline(PipelineType::Image).await
    }

    pub async fn get_text_pipeline(&self) -> Result<Pipeline, PipelineError> {
        self.get_or_create_graphics_pipeline(PipelineType::Text).await
    }

    pub async fn get_compute_pipeline(&self, name: &str) -> Result<Pipeline, PipelineError> {
        {
            let manager = self.manager.read().await;
            if let Some(existing) = manager.get_pipeline(name) {
                let shader_hash = self.get_cached_shader_hash(&format!("{}.comp", name)).await;
                return Ok(Pipeline {
                    pipeline: existing.get_pipeline(),
                    layout: existing.get_layout(),
                    bind_point: existing.get_bind_point(),
                    hash: shader_hash,
                    shader_stages: vec![vk::ShaderStageFlags::COMPUTE],
                });
            }
        }

        self.create_compute_pipeline(name).await
    }

    async fn get_or_create_graphics_pipeline(&self, pipeline_type: PipelineType) -> Result<Pipeline, PipelineError> {
        let spec = Self::get_pipeline_spec(pipeline_type);
        
        {
            let manager = self.manager.read().await;
            if let Some(existing) = manager.get_pipeline(spec.name) {
                let vertex_hash = self.get_cached_shader_hash(spec.vertex_shader).await;
                let fragment_hash = self.get_cached_shader_hash(spec.fragment_shader).await;
                let combined_hash = self.combine_hashes(&[vertex_hash, fragment_hash]);
                
                return Ok(Pipeline {
                    pipeline: existing.get_pipeline(),
                    layout: existing.get_layout(),
                    bind_point: existing.get_bind_point(),
                    hash: combined_hash,
                    shader_stages: vec![vk::ShaderStageFlags::VERTEX, vk::ShaderStageFlags::FRAGMENT],
                });
            }
        }

        self.create_graphics_pipeline(spec).await
    }

    async fn create_graphics_pipeline(&self, spec: PipelineSpec) -> Result<Pipeline, PipelineError> {
        let vertex_shader = self.load_shader(spec.vertex_shader).await?;
        let fragment_shader = self.load_shader(spec.fragment_shader).await?;
        let vertex_input_info = Self::create_vertex_input_info(spec.vertex_type);
        let combined_hash = self.calculate_shader_hash(&[&vertex_shader, &fragment_shader]);

        let mut manager = self.manager.write().await;
        let pipeline = manager.create_graphics_pipeline(
            spec.name,
            &vertex_shader,
            &fragment_shader,
            vertex_input_info,
        )?;

        Ok(Pipeline {
            pipeline: pipeline.get_pipeline(),
            layout: pipeline.get_layout(),
            bind_point: pipeline.get_bind_point(),
            hash: combined_hash,
            shader_stages: vec![vk::ShaderStageFlags::VERTEX, vk::ShaderStageFlags::FRAGMENT],
        })
    }

    async fn create_compute_pipeline(&self, name: &str) -> Result<Pipeline, PipelineError> {
        let compute_shader = self.load_shader(&format!("{}.comp", name)).await?;
        let shader_hash = self.calculate_shader_hash(&[&compute_shader]);

        let mut manager = self.manager.write().await;
        let pipeline = manager.create_compute_pipeline(name, &compute_shader)?;

        Ok(Pipeline {
            pipeline: pipeline.get_pipeline(),
            layout: pipeline.get_layout(),
            bind_point: pipeline.get_bind_point(),
            hash: shader_hash,
            shader_stages: vec![vk::ShaderStageFlags::COMPUTE],
        })
    }

    fn get_pipeline_spec(pipeline_type: PipelineType) -> PipelineSpec {
        match pipeline_type {
            PipelineType::Rect => PipelineSpec {
                name: "rect",
                vertex_shader: "rect.vert",
                fragment_shader: "rect.frag",
                vertex_type: VertexType::Standard,
            },
            PipelineType::Image => PipelineSpec {
                name: "image",
                vertex_shader: "image.vert",
                fragment_shader: "image.frag",
                vertex_type: VertexType::Standard,
            },
            PipelineType::Text => PipelineSpec {
                name: "text",
                vertex_shader: "text.vert",
                fragment_shader: "text.frag",
                vertex_type: VertexType::Text,
            },
        }
    }

    async fn load_shader(&self, name: &str) -> Result<Vec<u8>, PipelineError> {
        let shader_path = format!("resources/shaders/{}.spv", name);
        
        {
            let cache = self.shader_cache.read().await;
            if let Some(shader_data) = cache.get(name) {
                if self.is_shader_current(&shader_path, shader_data).await? {
                    return Ok(shader_data.content.clone());
                }
            }
        }

        let metadata = tokio::fs::metadata(&shader_path).await
            .map_err(|e| PipelineError::ShaderCompilationError(format!("Shader metadata error for {}: {}", name, e)))?;
        
        let shader_content = tokio::fs::read(&shader_path).await
            .map_err(|e| PipelineError::ShaderCompilationError(format!("Failed to read shader {}: {}", name, e)))?;

        let shader_hash = self.calculate_content_hash(&shader_content);
        let shader_data = ShaderData {
            content: shader_content.clone(),
            hash: shader_hash,
            last_modified: metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
        };

        {
            let mut cache = self.shader_cache.write().await;
            cache.insert(name.to_string(), shader_data);
        }

        Ok(shader_content)
    }

    async fn get_cached_shader_hash(&self, name: &str) -> u64 {
        let cache = self.shader_cache.read().await;
        cache.get(name).map(|data| data.hash).unwrap_or(0)
    }

    async fn is_shader_current(&self, path: &str, cached_data: &ShaderData) -> Result<bool, PipelineError> {
        if !self.hot_reload_enabled {
            return Ok(true);
        }

        let metadata = tokio::fs::metadata(path).await
            .map_err(|e| PipelineError::ShaderCompilationError(format!("Timestamp check failed: {}", e)))?;
        
        Ok(metadata.modified().unwrap_or(std::time::UNIX_EPOCH) <= cached_data.last_modified)
    }

    fn create_vertex_input_info(vertex_type: VertexType) -> vk::PipelineVertexInputStateCreateInfo {
        match vertex_type {
            VertexType::Standard => Self::create_standard_vertex_input(),
            VertexType::Text => Self::create_text_vertex_input(),
        }
    }

    fn create_standard_vertex_input() -> vk::PipelineVertexInputStateCreateInfo {
        let binding_descriptions = [vk::VertexInputBindingDescription::builder()
            .binding(0)
            .stride(std::mem::size_of::<crate::renderer::Vertex>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX)
            .build()];

        let attribute_descriptions = [
            vk::VertexInputAttributeDescription::builder()
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(0)
                .build(),
            vk::VertexInputAttributeDescription::builder()
                .binding(0)
                .location(1)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(12)
                .build(),
            vk::VertexInputAttributeDescription::builder()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32B32A32_SFLOAT)
                .offset(20)
                .build(),
        ];

        vk::PipelineVertexInputStateCreateInfo::builder()
            .vertex_binding_descriptions(&binding_descriptions)
            .vertex_attribute_descriptions(&attribute_descriptions)
            .build()
    }

    fn create_text_vertex_input() -> vk::PipelineVertexInputStateCreateInfo {
        let binding_descriptions = [vk::VertexInputBindingDescription::builder()
            .binding(0)
            .stride(std::mem::size_of::<TextVertex>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX)
            .build()];

        let attribute_descriptions = [
            vk::VertexInputAttributeDescription::builder()
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(0)
                .build(),
            vk::VertexInputAttributeDescription::builder()
                .binding(0)
                .location(1)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(8)
                .build(),
            vk::VertexInputAttributeDescription::builder()
                .binding(0)
                .location(2)
                .format(vk::Format::R32G32B32A32_SFLOAT)
                .offset(16)
                .build(),
        ];

        vk::PipelineVertexInputStateCreateInfo::builder()
            .vertex_binding_descriptions(&binding_descriptions)
            .vertex_attribute_descriptions(&attribute_descriptions)
            .build()
    }

    fn calculate_shader_hash(&self, shaders: &[&Vec<u8>]) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for shader in shaders {
            shader.hash(&mut hasher);
        }
        hasher.finish()
    }

    fn calculate_content_hash(&self, content: &[u8]) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        content.hash(&mut hasher);
        hasher.finish()
    }

    fn combine_hashes(&self, hashes: &[u64]) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hashes.hash(&mut hasher);
        hasher.finish()
    }

    pub async fn hot_reload_shaders(&self) -> Result<(), PipelineError> {
        if !self.hot_reload_enabled {
            return Ok(());
        }

        let pipeline_specs = [
            ("rect", ["rect.vert", "rect.frag"]),
            ("image", ["image.vert", "image.frag"]),
            ("text", ["text.vert", "text.frag"]),
        ];

        let mut modified_pipelines = Vec::with_capacity(pipeline_specs.len());

        for (pipeline_name, shader_names) in &pipeline_specs {
            let mut needs_reload = false;
            
            for shader_name in shader_names {
                let shader_path = format!("resources/shaders/{}.spv", shader_name);
                let cache = self.shader_cache.read().await;
                
                if let Some(cached_data) = cache.get(*shader_name) {
                    if !self.is_shader_current(&shader_path, cached_data).await? {
                        needs_reload = true;
                        break;
                    }
                } else {
                    needs_reload = true;
                    break;
                }
            }

            if needs_reload {
                modified_pipelines.push(*pipeline_name);
            }
        }

        if modified_pipelines.is_empty() {
            return Ok(());
        }

        {
            let mut shader_cache = self.shader_cache.write().await;
            for pipeline_name in &modified_pipelines {
                if let Some((_, shader_names)) = pipeline_specs.iter().find(|(name, _)| name == pipeline_name) {
                    for shader_name in shader_names {
                        shader_cache.remove(*shader_name);
                    }
                }
            }
        }

        let mut manager = self.manager.write().await;
        
        for pipeline_name in &modified_pipelines {
            if let Some((_, shader_names)) = pipeline_specs.iter().find(|(name, _)| name == pipeline_name) {
                let vertex_shader = self.load_shader(shader_names[0]).await?;
                let fragment_shader = self.load_shader(shader_names[1]).await?;
                
                manager.reload_pipeline(pipeline_name, Some(&vertex_shader), Some(&fragment_shader))?;
            }
        }

        log::info!("Hot reloaded {} pipelines: {:?}", modified_pipelines.len(), modified_pipelines);
        Ok(())
    }
}

impl Default for PipelineCache {
    fn default() -> Self {
        panic!("PipelineCache requires device and render pass to initialize");
    }
}