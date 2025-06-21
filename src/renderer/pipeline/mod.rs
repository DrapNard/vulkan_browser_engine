pub mod cache;

pub use cache::*;

use ash::vk;
use std::collections::HashMap;
use std::sync::Arc;

pub struct PipelineManager {
    device: Arc<ash::Device>,
    pipelines: HashMap<String, Pipeline>,
    pipeline_layout: vk::PipelineLayout,
    render_pass: vk::RenderPass,
}

pub struct Pipeline {
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    bind_point: vk::PipelineBindPoint,
}

impl PipelineBuilder {
    pub fn build_color_blending(&self) -> Result<vk::PipelineColorBlendStateCreateInfo> {
        let color_blend_attachment = vk::PipelineColorBlendAttachmentState::builder()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false)
            .build();

        let color_blend_attachments = vec![color_blend_attachment];
        
        Ok(vk::PipelineColorBlendStateCreateInfo::builder()
            .logic_op_enable(false)
            .logic_op(vk::LogicOp::COPY)
            .attachments(&color_blend_attachments)
            .blend_constants([0.0, 0.0, 0.0, 0.0])
            .build())
    }
}

impl PipelineManager {
    pub fn new(
        device: Arc<ash::Device>,
        render_pass: vk::RenderPass,
    ) -> Result<Self, PipelineError> {
        let pipeline_layout = Self::create_pipeline_layout(&device)?;

        Ok(Self {
            device,
            pipelines: HashMap::new(),
            pipeline_layout,
            render_pass,
        })
    }

    fn create_pipeline_layout(device: &ash::Device) -> Result<vk::PipelineLayout, PipelineError> {
        let descriptor_set_layouts = [];
        let push_constant_ranges = [
            vk::PushConstantRange::builder()
                .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
                .offset(0)
                .size(128)
                .build(),
        ];

        let layout_info = vk::PipelineLayoutCreateInfo::builder()
            .set_layouts(&descriptor_set_layouts)
            .push_constant_ranges(&push_constant_ranges);

        unsafe {
            device
                .create_pipeline_layout(&layout_info, None)
                .map_err(PipelineError::VulkanError)
        }
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

        let vert_stage_info = vk::PipelineShaderStageCreateInfo::builder()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vert_shader_module)
            .name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap());

        let frag_stage_info = vk::PipelineShaderStageCreateInfo::builder()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(frag_shader_module)
            .name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap());

        let shader_stages = [vert_stage_info.build(), frag_stage_info.build()];

        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::builder()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);

        let viewport_state = vk::PipelineViewportStateCreateInfo::builder()
            .viewport_count(1)
            .scissor_count(1);

        let rasterizer = vk::PipelineRasterizationStateCreateInfo::builder()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::CLOCKWISE)
            .depth_bias_enable(false);

        let multisampling = vk::PipelineMultisampleStateCreateInfo::builder()
            .sample_shading_enable(false)
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        let color_blend_attachment = vk::PipelineColorBlendAttachmentState::builder()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ZERO)
            .alpha_blend_op(vk::BlendOp::ADD)
            .build();

        let color_blending = vk::PipelineColorBlendStateCreateInfo::builder()
            .logic_op_enable(false)
            .logic_op(vk::LogicOp::COPY)
            .attachments(&[color_blend_attachment])
            .blend_constants([0.0, 0.0, 0.0, 0.0]);

        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state = vk::PipelineDynamicStateCreateInfo::builder()
            .dynamic_states(&dynamic_states);

        let pipeline_info = vk::GraphicsPipelineCreateInfo::builder()
            .stages(&shader_stages)
            .vertex_input_state(&vertex_input_info)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .color_blend_state(&color_blending)
            .dynamic_state(&dynamic_state)
            .layout(self.pipeline_layout)
            .render_pass(self.render_pass)
            .subpass(0);

        let pipeline = unsafe {
            self.device
                .create_graphics_pipelines(
                    vk::PipelineCache::null(),
                    &[pipeline_info.build()],
                    None,
                )
                .map_err(|e| PipelineError::VulkanError(e.1))?[0]
        };

        unsafe {
            self.device.destroy_shader_module(vert_shader_module, None);
            self.device.destroy_shader_module(frag_shader_module, None);
        }

        let pipeline_obj = Pipeline {
            pipeline,
            layout: self.pipeline_layout,
            bind_point: vk::PipelineBindPoint::GRAPHICS,
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

        let compute_stage_info = vk::PipelineShaderStageCreateInfo::builder()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(compute_shader_module)
            .name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap());

        let pipeline_info = vk::ComputePipelineCreateInfo::builder()
            .stage(compute_stage_info.build())
            .layout(self.pipeline_layout);

        let pipeline = unsafe {
            self.device
                .create_compute_pipelines(
                    vk::PipelineCache::null(),
                    &[pipeline_info.build()],
                    None,
                )
                .map_err(|e| PipelineError::VulkanError(e.1))?[0]
        };

        unsafe {
            self.device.destroy_shader_module(compute_shader_module, None);
        }

        let pipeline_obj = Pipeline {
            pipeline,
            layout: self.pipeline_layout,
            bind_point: vk::PipelineBindPoint::COMPUTE,
        };

        self.pipelines.insert(name.to_string(), pipeline_obj);
        Ok(self.pipelines.get(name).unwrap())
    }

    fn create_shader_module(&self, code: &[u8]) -> Result<vk::ShaderModule, PipelineError> {
        let create_info = vk::ShaderModuleCreateInfo::builder()
            .code(unsafe {
                std::slice::from_raw_parts(
                    code.as_ptr() as *const u32,
                    code.len() / std::mem::size_of::<u32>(),
                )
            });

        unsafe {
            self.device
                .create_shader_module(&create_info, None)
                .map_err(PipelineError::VulkanError)
        }
    }

    pub fn get_pipeline(&self, name: &str) -> Option<&Pipeline> {
        self.pipelines.get(name)
    }

    pub fn bind_pipeline(&self, cmd: vk::CommandBuffer, pipeline: &Pipeline) {
        unsafe {
            self.device.cmd_bind_pipeline(cmd, pipeline.bind_point, pipeline.pipeline);
        }
    }

    pub fn get_pipeline_layout(&self) -> vk::PipelineLayout {
        self.pipeline_layout
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
}

impl Drop for PipelineManager {
    fn drop(&mut self) {
        unsafe {
            for pipeline in self.pipelines.values() {
                self.device.destroy_pipeline(pipeline.pipeline, None);
            }
            self.device.destroy_pipeline_layout(self.pipeline_layout, None);
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
}

unsafe impl Send for PipelineManager {}
unsafe impl Sync for PipelineManager {}