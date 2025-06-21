use super::{Pipeline, PipelineError, PipelineManager};
use ash::vk;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::log;

pub struct PipelineCache {
    manager: Arc<RwLock<PipelineManager>>,
    shader_cache: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    hot_reload_enabled: bool,
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
        let manager = self.manager.read().await;
        if let Some(pipeline) = manager.get_pipeline("rect") {
            return Ok(Pipeline {
                pipeline: pipeline.get_pipeline(),
                layout: pipeline.get_layout(),
                bind_point: pipeline.get_bind_point(),
            });
        }
        drop(manager);

        self.create_rect_pipeline().await
    }

    pub async fn get_image_pipeline(&self) -> Result<Pipeline, PipelineError> {
        let manager = self.manager.read().await;
        if let Some(pipeline) = manager.get_pipeline("image") {
            return Ok(Pipeline {
                pipeline: pipeline.get_pipeline(),
                layout: pipeline.get_layout(),
                bind_point: pipeline.get_bind_point(),
            });
        }
        drop(manager);

        self.create_image_pipeline().await
    }

    pub async fn get_text_pipeline(&self) -> Result<Pipeline, PipelineError> {
        let manager = self.manager.read().await;
        if let Some(pipeline) = manager.get_pipeline("text") {
            return Ok(Pipeline {
                pipeline: pipeline.get_pipeline(),
                layout: pipeline.get_layout(),
                bind_point: pipeline.get_bind_point(),
            });
        }
        drop(manager);

        self.create_text_pipeline().await
    }

    async fn create_rect_pipeline(&self) -> Result<Pipeline, PipelineError> {
        let vertex_shader = self.load_shader("rect.vert").await?;
        let fragment_shader = self.load_shader("rect.frag").await?;

        let vertex_input_info = self.create_vertex_input_info();

        let mut manager = self.manager.write().await;
        let pipeline = manager.create_graphics_pipeline(
            "rect",
            &vertex_shader,
            &fragment_shader,
            vertex_input_info,
        )?;

        Ok(Pipeline {
            pipeline: pipeline.get_pipeline(),
            layout: pipeline.get_layout(),
            bind_point: pipeline.get_bind_point(),
        })
    }

    async fn create_image_pipeline(&self) -> Result<Pipeline, PipelineError> {
        let vertex_shader = self.load_shader("image.vert").await?;
        let fragment_shader = self.load_shader("image.frag").await?;

        let vertex_input_info = self.create_vertex_input_info();

        let mut manager = self.manager.write().await;
        let pipeline = manager.create_graphics_pipeline(
            "image",
            &vertex_shader,
            &fragment_shader,
            vertex_input_info,
        )?;

        Ok(Pipeline {
            pipeline: pipeline.get_pipeline(),
            layout: pipeline.get_layout(),
            bind_point: pipeline.get_bind_point(),
        })
    }

    async fn create_text_pipeline(&self) -> Result<Pipeline, PipelineError> {
        let vertex_shader = self.load_shader("text.vert").await?;
        let fragment_shader = self.load_shader("text.frag").await?;

        let vertex_input_info = self.create_text_vertex_input_info();

        let mut manager = self.manager.write().await;
        let pipeline = manager.create_graphics_pipeline(
            "text",
            &vertex_shader,
            &fragment_shader,
            vertex_input_info,
        )?;

        Ok(Pipeline {
            pipeline: pipeline.get_pipeline(),
            layout: pipeline.get_layout(),
            bind_point: pipeline.get_bind_point(),
        })
    }

    async fn load_shader(&self, name: &str) -> Result<Vec<u8>, PipelineError> {
        {
            let cache = self.shader_cache.read().await;
            if let Some(shader_code) = cache.get(name) {
                return Ok(shader_code.clone());
            }
        }

        let shader_path = format!("resources/shaders/{}.spv", name);
        let shader_code = tokio::fs::read(&shader_path).await
            .map_err(|e| PipelineError::ShaderCompilationError(e.to_string()))?;

        {
            let mut cache = self.shader_cache.write().await;
            cache.insert(name.to_string(), shader_code.clone());
        }

        Ok(shader_code)
    }

    fn create_vertex_input_info(&self) -> vk::PipelineVertexInputStateCreateInfo {
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

    fn create_text_vertex_input_info(&self) -> vk::PipelineVertexInputStateCreateInfo {
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

    pub async fn hot_reload_shaders(&self) -> Result<(), PipelineError> {
        if !self.hot_reload_enabled {
            return Ok(());
        }

        let mut shader_cache = self.shader_cache.write().await;
        shader_cache.clear();

        let mut manager = self.manager.write().await;
        
        let rect_vert = self.load_shader("rect.vert").await?;
        let rect_frag = self.load_shader("rect.frag").await?;
        manager.reload_pipeline("rect", Some(&rect_vert), Some(&rect_frag))?;

        let image_vert = self.load_shader("image.vert").await?;
        let image_frag = self.load_shader("image.frag").await?;
        manager.reload_pipeline("image", Some(&image_vert), Some(&image_frag))?;

        let text_vert = self.load_shader("text.vert").await?;
        let text_frag = self.load_shader("text.frag").await?;
        manager.reload_pipeline("text", Some(&text_vert), Some(&text_frag))?;

        log::info!("Hot reloaded all shaders");
        Ok(())
    }

    pub async fn get_compute_pipeline(&self, name: &str) -> Result<Pipeline, PipelineError> {
        let manager = self.manager.read().await;
        if let Some(pipeline) = manager.get_pipeline(name) {
            return Ok(Pipeline {
                pipeline: pipeline.get_pipeline(),
                layout: pipeline.get_layout(),
                bind_point: pipeline.get_bind_point(),
            });
        }
        drop(manager);

        self.create_compute_pipeline(name).await
    }

    async fn create_compute_pipeline(&self, name: &str) -> Result<Pipeline, PipelineError> {
        let compute_shader = self.load_shader(&format!("{}.comp", name)).await?;

        let mut manager = self.manager.write().await;
        let pipeline = manager.create_compute_pipeline(name, &compute_shader)?;

        Ok(Pipeline {
            pipeline: pipeline.get_pipeline(),
            layout: pipeline.get_layout(),
            bind_point: pipeline.get_bind_point(),
        })
    }
}

#[derive(Debug, Clone)]
struct TextVertex {
    position: [f32; 2],
    tex_coord: [f32; 2],
    color: [f32; 4],
}

impl Default for PipelineCache {
    fn default() -> Self {
        panic!("PipelineCache requires device and render pass to initialize");
    }
}