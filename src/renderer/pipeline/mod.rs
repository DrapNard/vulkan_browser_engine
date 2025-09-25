pub mod cache;

pub use cache::*;

use ash::vk;
use std::collections::HashMap;
use std::sync::Arc;

pub struct PipelineManager {
    device: Arc<ash::Device>,
    pipelines: HashMap<String, Pipeline>,
    pipeline_layouts: HashMap<String, vk::PipelineLayout>,
    shader_cache: HashMap<u64, vk::ShaderModule>,
    default_layout: vk::PipelineLayout,
    render_pass: vk::RenderPass,
}

pub struct Pipeline {
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    bind_point: vk::PipelineBindPoint,
    shader_stages: Vec<vk::ShaderStageFlags>,
    hash: u64,
}

pub struct PipelineBuilder {
    device: Arc<ash::Device>,
    vertex_input: Option<vk::PipelineVertexInputStateCreateInfo>,
    input_assembly: Option<vk::PipelineInputAssemblyStateCreateInfo>,
    rasterization: Option<vk::PipelineRasterizationStateCreateInfo>,
    multisample: Option<vk::PipelineMultisampleStateCreateInfo>,
    color_blend: Option<vk::PipelineColorBlendStateCreateInfo>,
    depth_stencil: Option<vk::PipelineDepthStencilStateCreateInfo>,
    dynamic_states: Vec<vk::DynamicState>,
    shader_stages: Vec<vk::PipelineShaderStageCreateInfo>,
    layout: Option<vk::PipelineLayout>,
    render_pass: Option<vk::RenderPass>,
    subpass: u32,
}

impl PipelineBuilder {
    pub fn new(device: Arc<ash::Device>) -> Self {
        Self {
            device,
            vertex_input: None,
            input_assembly: None,
            rasterization: None,
            multisample: None,
            color_blend: None,
            depth_stencil: None,
            dynamic_states: Vec::new(),
            shader_stages: Vec::new(),
            layout: None,
            render_pass: None,
            subpass: 0,
        }
    }

    pub fn vertex_input(mut self, vertex_input: vk::PipelineVertexInputStateCreateInfo) -> Self {
        self.vertex_input = Some(vertex_input);
        self
    }

    pub fn input_assembly(mut self, topology: vk::PrimitiveTopology, restart: bool) -> Self {
        self.input_assembly = Some(
            vk::PipelineInputAssemblyStateCreateInfo::builder()
                .topology(topology)
                .primitive_restart_enable(restart)
                .build(),
        );
        self
    }

    pub fn rasterization(
        mut self,
        polygon_mode: vk::PolygonMode,
        cull_mode: vk::CullModeFlags,
        front_face: vk::FrontFace,
    ) -> Self {
        self.rasterization = Some(
            vk::PipelineRasterizationStateCreateInfo::builder()
                .depth_clamp_enable(false)
                .rasterizer_discard_enable(false)
                .polygon_mode(polygon_mode)
                .line_width(1.0)
                .cull_mode(cull_mode)
                .front_face(front_face)
                .depth_bias_enable(false)
                .build(),
        );
        self
    }

    pub fn multisample(mut self, samples: vk::SampleCountFlags, sample_shading: bool) -> Self {
        self.multisample = Some(
            vk::PipelineMultisampleStateCreateInfo::builder()
                .sample_shading_enable(sample_shading)
                .rasterization_samples(samples)
                .min_sample_shading(if sample_shading { 0.2 } else { 0.0 })
                .build(),
        );
        self
    }

    pub fn build_color_blending(
        &self,
        blend_enable: bool,
    ) -> Result<vk::PipelineColorBlendStateCreateInfo, PipelineError> {
        let color_blend_attachment = if blend_enable {
            vk::PipelineColorBlendAttachmentState::builder()
                .color_write_mask(vk::ColorComponentFlags::RGBA)
                .blend_enable(true)
                .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
                .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                .color_blend_op(vk::BlendOp::ADD)
                .src_alpha_blend_factor(vk::BlendFactor::ONE)
                .dst_alpha_blend_factor(vk::BlendFactor::ZERO)
                .alpha_blend_op(vk::BlendOp::ADD)
                .build()
        } else {
            vk::PipelineColorBlendAttachmentState::builder()
                .color_write_mask(vk::ColorComponentFlags::RGBA)
                .blend_enable(false)
                .build()
        };

        let color_blend_attachments = vec![color_blend_attachment];

        Ok(vk::PipelineColorBlendStateCreateInfo::builder()
            .logic_op_enable(false)
            .logic_op(vk::LogicOp::COPY)
            .attachments(&color_blend_attachments)
            .blend_constants([0.0, 0.0, 0.0, 0.0])
            .build())
    }

    pub fn color_blending(mut self, blend_enable: bool) -> Result<Self, PipelineError> {
        self.color_blend = Some(self.build_color_blending(blend_enable)?);
        Ok(self)
    }

    pub fn depth_stencil(
        mut self,
        depth_test: bool,
        depth_write: bool,
        compare_op: vk::CompareOp,
    ) -> Self {
        self.depth_stencil = Some(
            vk::PipelineDepthStencilStateCreateInfo::builder()
                .depth_test_enable(depth_test)
                .depth_write_enable(depth_write)
                .depth_compare_op(compare_op)
                .depth_bounds_test_enable(false)
                .stencil_test_enable(false)
                .min_depth_bounds(0.0)
                .max_depth_bounds(1.0)
                .build(),
        );
        self
    }

    pub fn dynamic_states(mut self, states: &[vk::DynamicState]) -> Self {
        self.dynamic_states = states.to_vec();
        self
    }

    pub fn add_shader_stage(
        mut self,
        stage: vk::ShaderStageFlags,
        module: vk::ShaderModule,
        entry_point: &str,
    ) -> Result<Self, PipelineError> {
        let entry_name = std::ffi::CString::new(entry_point).map_err(|_| {
            PipelineError::ShaderCompilationError("Invalid entry point name".to_string())
        })?;

        let stage_info = vk::PipelineShaderStageCreateInfo::builder()
            .stage(stage)
            .module(module)
            .name(&entry_name)
            .build();

        self.shader_stages.push(stage_info);
        Ok(self)
    }

    pub fn layout(mut self, layout: vk::PipelineLayout) -> Self {
        self.layout = Some(layout);
        self
    }

    pub fn render_pass(mut self, render_pass: vk::RenderPass, subpass: u32) -> Self {
        self.render_pass = Some(render_pass);
        self.subpass = subpass;
        self
    }

    pub fn build_graphics(self) -> Result<vk::Pipeline, PipelineError> {
        let vertex_input = self
            .vertex_input
            .unwrap_or_else(|| vk::PipelineVertexInputStateCreateInfo::builder().build());

        let input_assembly = self.input_assembly.unwrap_or_else(|| {
            vk::PipelineInputAssemblyStateCreateInfo::builder()
                .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
                .primitive_restart_enable(false)
                .build()
        });

        let viewport_state = vk::PipelineViewportStateCreateInfo::builder()
            .viewport_count(1)
            .scissor_count(1)
            .build();

        let rasterization = self.rasterization.unwrap_or_else(|| {
            vk::PipelineRasterizationStateCreateInfo::builder()
                .depth_clamp_enable(false)
                .rasterizer_discard_enable(false)
                .polygon_mode(vk::PolygonMode::FILL)
                .line_width(1.0)
                .cull_mode(vk::CullModeFlags::BACK)
                .front_face(vk::FrontFace::CLOCKWISE)
                .depth_bias_enable(false)
                .build()
        });

        let multisample = self.multisample.unwrap_or_else(|| {
            vk::PipelineMultisampleStateCreateInfo::builder()
                .sample_shading_enable(false)
                .rasterization_samples(vk::SampleCountFlags::TYPE_1)
                .build()
        });

        let color_blend_attachments = vec![vk::PipelineColorBlendAttachmentState::builder()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false)
            .build()];

        let color_blending = self.color_blend.unwrap_or_else(|| {
            vk::PipelineColorBlendStateCreateInfo::builder()
                .logic_op_enable(false)
                .logic_op(vk::LogicOp::COPY)
                .attachments(&color_blend_attachments)
                .blend_constants([0.0, 0.0, 0.0, 0.0])
                .build()
        });

        let dynamic_state = if !self.dynamic_states.is_empty() {
            Some(
                vk::PipelineDynamicStateCreateInfo::builder()
                    .dynamic_states(&self.dynamic_states)
                    .build(),
            )
        } else {
            None
        };

        let layout = self.layout.ok_or(PipelineError::PipelineCreationError(
            "Pipeline layout not set".to_string(),
        ))?;
        let render_pass = self
            .render_pass
            .ok_or(PipelineError::PipelineCreationError(
                "Render pass not set".to_string(),
            ))?;

        let mut pipeline_info = vk::GraphicsPipelineCreateInfo::builder()
            .stages(&self.shader_stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterization)
            .multisample_state(&multisample)
            .color_blend_state(&color_blending)
            .layout(layout)
            .render_pass(render_pass)
            .subpass(self.subpass);

        if let Some(ref dynamic) = dynamic_state {
            pipeline_info = pipeline_info.dynamic_state(dynamic);
        }

        if let Some(ref depth_stencil) = self.depth_stencil {
            pipeline_info = pipeline_info.depth_stencil_state(depth_stencil);
        }

        unsafe {
            self.device
                .create_graphics_pipelines(
                    vk::PipelineCache::null(),
                    &[pipeline_info.build()],
                    None,
                )
                .map_err(|e| PipelineError::VulkanError(e.1))
                .map(|pipelines| pipelines[0])
        }
    }

    pub fn build_compute(self) -> Result<vk::Pipeline, PipelineError> {
        if self.shader_stages.len() != 1 {
            return Err(PipelineError::PipelineCreationError(
                "Compute pipeline requires exactly one shader stage".to_string(),
            ));
        }

        let compute_stage = &self.shader_stages[0];
        if compute_stage.stage != vk::ShaderStageFlags::COMPUTE {
            return Err(PipelineError::PipelineCreationError(
                "Compute pipeline requires compute shader stage".to_string(),
            ));
        }

        let layout = self.layout.ok_or(PipelineError::PipelineCreationError(
            "Pipeline layout not set".to_string(),
        ))?;

        let pipeline_info = vk::ComputePipelineCreateInfo::builder()
            .stage(*compute_stage)
            .layout(layout)
            .build();

        unsafe {
            self.device
                .create_compute_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
                .map_err(|e| PipelineError::VulkanError(e.1))
                .map(|pipelines| pipelines[0])
        }
    }
}

impl PipelineManager {
    pub fn new(
        device: Arc<ash::Device>,
        render_pass: vk::RenderPass,
    ) -> Result<Self, PipelineError> {
        let default_layout = Self::create_default_pipeline_layout(&device)?;

        Ok(Self {
            device,
            pipelines: HashMap::new(),
            pipeline_layouts: HashMap::new(),
            shader_cache: HashMap::new(),
            default_layout,
            render_pass,
        })
    }

    fn create_default_pipeline_layout(
        device: &ash::Device,
    ) -> Result<vk::PipelineLayout, PipelineError> {
        let descriptor_set_layouts = [];
        let push_constant_ranges = [vk::PushConstantRange::builder()
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
            .offset(0)
            .size(128)
            .build()];

        let layout_info = vk::PipelineLayoutCreateInfo::builder()
            .set_layouts(&descriptor_set_layouts)
            .push_constant_ranges(&push_constant_ranges);

        unsafe {
            device
                .create_pipeline_layout(&layout_info, None)
                .map_err(PipelineError::VulkanError)
        }
    }

    pub fn create_pipeline_layout(
        &mut self,
        name: &str,
        descriptor_set_layouts: &[vk::DescriptorSetLayout],
        push_constant_ranges: &[vk::PushConstantRange],
    ) -> Result<vk::PipelineLayout, PipelineError> {
        let layout_info = vk::PipelineLayoutCreateInfo::builder()
            .set_layouts(descriptor_set_layouts)
            .push_constant_ranges(push_constant_ranges);

        let layout = unsafe {
            self.device
                .create_pipeline_layout(&layout_info, None)
                .map_err(PipelineError::VulkanError)?
        };

        self.pipeline_layouts.insert(name.to_string(), layout);
        Ok(layout)
    }

    fn validate_shader_bytecode(code: &[u8]) -> Result<(), PipelineError> {
        if code.len() % 4 != 0 {
            return Err(PipelineError::ShaderCompilationError(
                "Shader bytecode length must be multiple of 4".to_string(),
            ));
        }

        if code.len() < 20 {
            return Err(PipelineError::ShaderCompilationError(
                "Shader bytecode too short".to_string(),
            ));
        }

        let magic = u32::from_le_bytes([code[0], code[1], code[2], code[3]]);
        if magic != 0x07230203 {
            return Err(PipelineError::ShaderCompilationError(
                "Invalid SPIR-V magic number".to_string(),
            ));
        }

        Ok(())
    }

    fn calculate_shader_hash(code: &[u8]) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        code.hash(&mut hasher);
        hasher.finish()
    }

    fn create_shader_module(&mut self, code: &[u8]) -> Result<vk::ShaderModule, PipelineError> {
        Self::validate_shader_bytecode(code)?;

        let hash = Self::calculate_shader_hash(code);

        if let Some(&cached_module) = self.shader_cache.get(&hash) {
            return Ok(cached_module);
        }

        let create_info = vk::ShaderModuleCreateInfo::builder().code(unsafe {
            std::slice::from_raw_parts(
                code.as_ptr() as *const u32,
                code.len() / std::mem::size_of::<u32>(),
            )
        });

        let module = unsafe {
            self.device
                .create_shader_module(&create_info, None)
                .map_err(PipelineError::VulkanError)?
        };

        self.shader_cache.insert(hash, module);
        Ok(module)
    }

    pub fn create_graphics_pipeline(
        &mut self,
        name: &str,
        vertex_shader: &[u8],
        fragment_shader: &[u8],
        vertex_input_info: vk::PipelineVertexInputStateCreateInfo,
    ) -> Result<&Pipeline, PipelineError> {
        let vert_shader_module = self.create_shader_module(vertex_shader)?;
        let frag_shader_module = self.create_shader_module(fragment_shader)?;

        let pipeline = PipelineBuilder::new(self.device.clone())
            .add_shader_stage(vk::ShaderStageFlags::VERTEX, vert_shader_module, "main")?
            .add_shader_stage(vk::ShaderStageFlags::FRAGMENT, frag_shader_module, "main")?
            .vertex_input(vertex_input_info)
            .input_assembly(vk::PrimitiveTopology::TRIANGLE_LIST, false)
            .rasterization(
                vk::PolygonMode::FILL,
                vk::CullModeFlags::BACK,
                vk::FrontFace::CLOCKWISE,
            )
            .multisample(vk::SampleCountFlags::TYPE_1, false)
            .color_blending(true)?
            .dynamic_states(&[vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR])
            .layout(self.default_layout)
            .render_pass(self.render_pass, 0)
            .build_graphics()?;

        let stages = vec![vk::ShaderStageFlags::VERTEX, vk::ShaderStageFlags::FRAGMENT];
        let hash = Self::calculate_pipeline_hash(&stages, &vertex_input_info);

        let pipeline_obj = Pipeline {
            pipeline,
            layout: self.default_layout,
            bind_point: vk::PipelineBindPoint::GRAPHICS,
            shader_stages: stages,
            hash,
        };

        self.pipelines.insert(name.to_string(), pipeline_obj);
        Ok(self.pipelines.get(name).unwrap())
    }

    pub fn create_compute_pipeline(
        &mut self,
        name: &str,
        compute_shader: &[u8],
    ) -> Result<&Pipeline, PipelineError> {
        let compute_shader_module = self.create_shader_module(compute_shader)?;

        let pipeline = PipelineBuilder::new(self.device.clone())
            .add_shader_stage(vk::ShaderStageFlags::COMPUTE, compute_shader_module, "main")?
            .layout(self.default_layout)
            .build_compute()?;

        let stages = vec![vk::ShaderStageFlags::COMPUTE];
        let hash = Self::calculate_pipeline_hash(
            &stages,
            &vk::PipelineVertexInputStateCreateInfo::default(),
        );

        let pipeline_obj = Pipeline {
            pipeline,
            layout: self.default_layout,
            bind_point: vk::PipelineBindPoint::COMPUTE,
            shader_stages: stages,
            hash,
        };

        self.pipelines.insert(name.to_string(), pipeline_obj);
        Ok(self.pipelines.get(name).unwrap())
    }

    fn calculate_pipeline_hash(
        stages: &[vk::ShaderStageFlags],
        vertex_input: &vk::PipelineVertexInputStateCreateInfo,
    ) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        stages.hash(&mut hasher);
        vertex_input
            .vertex_binding_description_count
            .hash(&mut hasher);
        vertex_input
            .vertex_attribute_description_count
            .hash(&mut hasher);
        hasher.finish()
    }

    pub fn get_pipeline(&self, name: &str) -> Option<&Pipeline> {
        self.pipelines.get(name)
    }

    pub fn get_pipeline_layout(&self, name: &str) -> Option<vk::PipelineLayout> {
        self.pipeline_layouts.get(name).copied()
    }

    pub fn get_default_layout(&self) -> vk::PipelineLayout {
        self.default_layout
    }

    pub fn bind_pipeline(&self, cmd: vk::CommandBuffer, pipeline: &Pipeline) {
        unsafe {
            self.device
                .cmd_bind_pipeline(cmd, pipeline.bind_point, pipeline.pipeline);
        }
    }

    pub fn reload_pipeline(
        &mut self,
        name: &str,
        vertex_shader: Option<&[u8]>,
        fragment_shader: Option<&[u8]>,
    ) -> Result<(), PipelineError> {
        if let Some(pipeline) = self.pipelines.remove(name) {
            unsafe {
                self.device.destroy_pipeline(pipeline.pipeline, None);
            }
        }

        if let (Some(vert), Some(frag)) = (vertex_shader, fragment_shader) {
            let vertex_input_info = vk::PipelineVertexInputStateCreateInfo::builder().build();
            self.create_graphics_pipeline(name, vert, frag, vertex_input_info)?;
        }

        Ok(())
    }

    pub fn clear_cache(&mut self) {
        for &module in self.shader_cache.values() {
            unsafe {
                self.device.destroy_shader_module(module, None);
            }
        }
        self.shader_cache.clear();
    }

    pub fn get_cache_size(&self) -> usize {
        self.shader_cache.len()
    }

    pub fn pipeline_exists(&self, name: &str) -> bool {
        self.pipelines.contains_key(name)
    }

    pub fn remove_pipeline(&mut self, name: &str) -> Result<(), PipelineError> {
        if let Some(pipeline) = self.pipelines.remove(name) {
            unsafe {
                self.device.destroy_pipeline(pipeline.pipeline, None);
            }
            Ok(())
        } else {
            Err(PipelineError::PipelineNotFound(name.to_string()))
        }
    }
}

impl Pipeline {
    pub fn get_pipeline(&self) -> vk::Pipeline {
        self.pipeline
    }

    pub fn get_layout(&self) -> vk::PipelineLayout {
        self.layout
    }

    pub fn get_bind_point(&self) -> vk::PipelineBindPoint {
        self.bind_point
    }

    pub fn get_shader_stages(&self) -> &[vk::ShaderStageFlags] {
        &self.shader_stages
    }

    pub fn get_hash(&self) -> u64 {
        self.hash
    }

    pub fn is_graphics_pipeline(&self) -> bool {
        self.bind_point == vk::PipelineBindPoint::GRAPHICS
    }

    pub fn is_compute_pipeline(&self) -> bool {
        self.bind_point == vk::PipelineBindPoint::COMPUTE
    }
}

impl Drop for PipelineManager {
    fn drop(&mut self) {
        unsafe {
            for pipeline in self.pipelines.values() {
                self.device.destroy_pipeline(pipeline.pipeline, None);
            }

            for &layout in self.pipeline_layouts.values() {
                self.device.destroy_pipeline_layout(layout, None);
            }

            for &module in self.shader_cache.values() {
                self.device.destroy_shader_module(module, None);
            }

            self.device
                .destroy_pipeline_layout(self.default_layout, None);
        }
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        // Pipeline cleanup is handled by PipelineManager
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("Vulkan error: {0}")]
    VulkanError(vk::Result),
    #[error("Shader compilation error: {0}")]
    ShaderCompilationError(String),
    #[error("Pipeline creation error: {0}")]
    PipelineCreationError(String),
    #[error("Pipeline not found: {0}")]
    PipelineNotFound(String),
    #[error("Invalid shader stage configuration: {0}")]
    InvalidShaderStage(String),
    #[error("Cache error: {0}")]
    CacheError(String),
}

unsafe impl Send for PipelineManager {}
unsafe impl Sync for PipelineManager {}
unsafe impl Send for Pipeline {}
unsafe impl Sync for Pipeline {}
