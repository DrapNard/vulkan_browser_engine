use std::sync::Arc;
use parking_lot::{Mutex, RwLock};
use ash::{Device, Instance, Entry};
use ash::vk;
use gpu_allocator::vulkan::{Allocator, AllocatorCreateDesc};
use thiserror::Error;
use dashmap::DashMap;
use smallvec::SmallVec;

pub mod device;
pub mod command;
pub mod shaders;

use device::{VulkanDevice, DeviceError};
use command::{CommandManager, CommandError};
use shaders::{ShaderManager, ShaderError};

use crate::core::{dom::Document, layout::LayoutEngine};
use crate::BrowserConfig;

#[derive(Error, Debug)]
pub enum VulkanError {
    #[error("Device creation failed: {0}")]
    DeviceCreation(String),
    #[error("Surface creation failed: {0}")]
    SurfaceCreation(String),
    #[error("Swapchain creation failed: {0}")]
    SwapchainCreation(String),
    #[error("Memory allocation failed: {0}")]
    MemoryAllocation(String),
    #[error("Pipeline creation failed: {0}")]
    PipelineCreation(String),
    #[error("Command buffer error: {0}")]
    CommandBuffer(String),
    #[error("Shader compilation failed: {0}")]
    ShaderCompilation(String),
    #[error("Device error: {0}")]
    Device(#[from] DeviceError),
    #[error("Command error: {0}")]
    Command(#[from] CommandError),
    #[error("Shader error: {0}")]
    Shader(#[from] ShaderError),
}

pub type Result<T> = std::result::Result<T, VulkanError>;

#[derive(Debug, Clone, Copy)]
pub struct RenderStats {
    pub frame_time_ms: f32,
    pub draw_calls: u32,
    pub triangles: u32,
    pub vertices: u32,
    pub memory_used_mb: u32,
    pub pipeline_switches: u32,
}

impl Default for RenderStats {
    fn default() -> Self {
        Self {
            frame_time_ms: 0.0,
            draw_calls: 0,
            triangles: 0,
            vertices: 0,
            memory_used_mb: 0,
            pipeline_switches: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RenderCommand {
    pub pipeline_id: u64,
    pub vertex_buffer: vk::Buffer,
    pub index_buffer: vk::Buffer,
    pub descriptor_sets: SmallVec<[vk::DescriptorSet; 4]>,
    pub index_count: u32,
    pub vertex_offset: u32,
    pub instance_count: u32,
}

#[derive(Debug, Clone)]
pub struct SwapchainData {
    pub swapchain: vk::SwapchainKHR,
    pub images: Vec<vk::Image>,
    pub image_views: Vec<vk::ImageView>,
    pub framebuffers: Vec<vk::Framebuffer>,
    pub format: vk::Format,
    pub extent: vk::Extent2D,
    pub current_index: u32,
}

struct MemoryTracker {
    allocated_bytes: std::sync::atomic::AtomicU64,
    peak_bytes: std::sync::atomic::AtomicU64,
}

impl MemoryTracker {
    fn new() -> Self {
        Self {
            allocated_bytes: std::sync::atomic::AtomicU64::new(0),
            peak_bytes: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn allocate(&self, bytes: u64) {
        let current = self.allocated_bytes.fetch_add(bytes, std::sync::atomic::Ordering::Relaxed) + bytes;
        let peak = self.peak_bytes.load(std::sync::atomic::Ordering::Relaxed);
        if current > peak {
            self.peak_bytes.store(current, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn deallocate(&self, bytes: u64) {
        self.allocated_bytes.fetch_sub(bytes, std::sync::atomic::Ordering::Relaxed);
    }

    fn current_usage(&self) -> u64 {
        self.allocated_bytes.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn peak_usage(&self) -> u64 {
        self.peak_bytes.load(std::sync::atomic::Ordering::Relaxed)
    }
}

struct ResourceManager {
    pipelines: DashMap<u64, vk::Pipeline>,
    descriptor_set_layouts: DashMap<String, vk::DescriptorSetLayout>,
    uniform_buffers: DashMap<String, (vk::Buffer, gpu_allocator::vulkan::Allocation)>,
    vertex_buffers: DashMap<u64, (vk::Buffer, gpu_allocator::vulkan::Allocation)>,
    index_buffers: DashMap<u64, (vk::Buffer, gpu_allocator::vulkan::Allocation)>,
    textures: DashMap<u64, (vk::Image, vk::ImageView, gpu_allocator::vulkan::Allocation)>,
}

impl ResourceManager {
    fn new() -> Self {
        Self {
            pipelines: DashMap::with_capacity(64),
            descriptor_set_layouts: DashMap::with_capacity(32),
            uniform_buffers: DashMap::with_capacity(128),
            vertex_buffers: DashMap::with_capacity(256),
            index_buffers: DashMap::with_capacity(256),
            textures: DashMap::with_capacity(512),
        }
    }

    unsafe fn cleanup(&self, device: &Device) {
        for entry in self.pipelines.iter() {
            device.destroy_pipeline(*entry.value(), None);
        }
        for entry in self.descriptor_set_layouts.iter() {
            device.destroy_descriptor_set_layout(*entry.value(), None);
        }
    }
}

pub struct VulkanRenderer {
    entry: Entry,
    instance: Instance,
    device: Arc<VulkanDevice>,
    allocator: Arc<Mutex<Allocator>>,
    surface: vk::SurfaceKHR,
    surface_loader: ash::extensions::khr::Surface,
    swapchain_loader: ash::extensions::khr::Swapchain,
    swapchain_data: Arc<RwLock<SwapchainData>>,
    command_manager: Arc<CommandManager>,
    shader_manager: Arc<ShaderManager>,
    render_pass: vk::RenderPass,
    pipeline_cache: vk::PipelineCache,
    descriptor_pool: vk::DescriptorPool,
    resources: ResourceManager,
    memory_tracker: MemoryTracker,
    frame_index: std::sync::atomic::AtomicU32,
    stats: Arc<RwLock<RenderStats>>,
    command_batches: Arc<RwLock<Vec<RenderCommand>>>,
}

impl VulkanRenderer {
    pub async fn new(config: &BrowserConfig) -> Result<Self> {
        let entry = unsafe { Entry::load() }
            .map_err(|e| VulkanError::DeviceCreation(e.to_string()))?;

        let instance = Self::create_instance(&entry)?;
        let surface = vk::SurfaceKHR::null();
        let surface_loader = ash::extensions::khr::Surface::new(&entry, &instance);
        
        let device = Arc::new(VulkanDevice::new(&entry, &instance, surface, &surface_loader).await?);
        
        let allocator = Self::create_allocator(&instance, &device)?;
        let swapchain_loader = ash::extensions::khr::Swapchain::new(&instance, device.logical_device());
        let command_manager = Arc::new(CommandManager::new(device.clone()).await?);
        let shader_manager = Arc::new(ShaderManager::new(device.clone()).await?);
        
        let render_pass = Self::create_render_pass(device.logical_device())?;
        let pipeline_cache = Self::create_pipeline_cache(device.logical_device())?;
        let descriptor_pool = Self::create_descriptor_pool(device.logical_device())?;
        
        let swapchain_data = Arc::new(RwLock::new(SwapchainData {
            swapchain: vk::SwapchainKHR::null(),
            images: Vec::with_capacity(3),
            image_views: Vec::with_capacity(3),
            framebuffers: Vec::with_capacity(3),
            format: vk::Format::B8G8R8A8_SRGB,
            extent: vk::Extent2D { 
                width: config.viewport_width.max(1), 
                height: config.viewport_height.max(1) 
            },
            current_index: 0,
        }));

        Ok(Self {
            entry,
            instance,
            device,
            allocator,
            surface,
            surface_loader,
            swapchain_loader,
            swapchain_data,
            command_manager,
            shader_manager,
            render_pass,
            pipeline_cache,
            descriptor_pool,
            resources: ResourceManager::new(),
            memory_tracker: MemoryTracker::new(),
            frame_index: std::sync::atomic::AtomicU32::new(0),
            stats: Arc::new(RwLock::new(RenderStats::default())),
            command_batches: Arc::new(RwLock::new(Vec::with_capacity(1024))),
        })
    }

    fn create_allocator(instance: &Instance, device: &Arc<VulkanDevice>) -> Result<Arc<Mutex<Allocator>>> {
        let allocator_desc = AllocatorCreateDesc {
            instance: instance.clone(),
            device: device.logical_device().clone(),
            physical_device: device.physical_device(),
            debug_settings: gpu_allocator::AllocatorDebugSettings {
                log_memory_information: cfg!(debug_assertions),
                log_leaks_on_shutdown: cfg!(debug_assertions),
                store_stack_traces: false,
                log_allocations: false,
                log_frees: false,
                log_stack_traces: false,
            },
            buffer_device_address: device.capabilities().supports_timeline_semaphores,
            allocation_sizes: gpu_allocator::AllocationSizes::default(),
        };
        
        let allocator = Allocator::new(&allocator_desc)
            .map_err(|e| VulkanError::MemoryAllocation(e.to_string()))?;
            
        Ok(Arc::new(Mutex::new(allocator)))
    }

    fn create_instance(entry: &Entry) -> Result<Instance> {
        let app_info = vk::ApplicationInfo::builder()
            .application_name(unsafe { c"Vulkan Browser" })
            .application_version(vk::make_api_version(0, 1, 0, 0))
            .engine_name(unsafe { c"VulkanBrowserEngine" })
            .engine_version(vk::make_api_version(0, 1, 0, 0))
            .api_version(vk::API_VERSION_1_3);

        let mut extension_names = vec![
            ash::extensions::khr::Surface::name().as_ptr(),
        ];

        let mut layer_names = Vec::new();

        if cfg!(debug_assertions) {
            extension_names.push(ash::extensions::ext::DebugUtils::name().as_ptr());
            layer_names.push(unsafe { 
                c"VK_LAYER_KHRONOS_validation".as_ptr() 
            });
        }

        let create_info = vk::InstanceCreateInfo::builder()
            .application_info(&app_info)
            .enabled_extension_names(&extension_names)
            .enabled_layer_names(&layer_names);

        unsafe {
            entry.create_instance(&create_info, None)
                .map_err(|e| VulkanError::DeviceCreation(e.to_string()))
        }
    }

    fn create_render_pass(device: &Device) -> Result<vk::RenderPass> {
        let attachments = [
            vk::AttachmentDescription::builder()
                .format(vk::Format::B8G8R8A8_SRGB)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .build(),
            vk::AttachmentDescription::builder()
                .format(vk::Format::D32_SFLOAT)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::DONT_CARE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)
                .build(),
        ];

        let color_attachment_refs = [vk::AttachmentReference::builder()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .build()];

        let depth_attachment_ref = vk::AttachmentReference::builder()
            .attachment(1)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)
            .build();
        
        let subpasses = [vk::SubpassDescription::builder()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_attachment_refs)
            .depth_stencil_attachment(&depth_attachment_ref)
            .build()];

        let dependencies = [vk::SubpassDependency::builder()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS)
            .src_access_mask(vk::AccessFlags::empty())
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS)
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
            .build()];

        let render_pass_info = vk::RenderPassCreateInfo::builder()
            .attachments(&attachments)
            .subpasses(&subpasses)
            .dependencies(&dependencies);

        unsafe {
            device.create_render_pass(&render_pass_info, None)
                .map_err(|e| VulkanError::PipelineCreation(e.to_string()))
        }
    }

    fn create_pipeline_cache(device: &Device) -> Result<vk::PipelineCache> {
        let cache_info = vk::PipelineCacheCreateInfo::builder();
        
        unsafe {
            device.create_pipeline_cache(&cache_info, None)
                .map_err(|e| VulkanError::PipelineCreation(e.to_string()))
        }
    }

    fn create_descriptor_pool(device: &Device) -> Result<vk::DescriptorPool> {
        let pool_sizes = [
            vk::DescriptorPoolSize::builder()
                .ty(vk::DescriptorType::UNIFORM_BUFFER)
                .descriptor_count(1000)
                .build(),
            vk::DescriptorPoolSize::builder()
                .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1000)
                .build(),
            vk::DescriptorPoolSize::builder()
                .ty(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(500)
                .build(),
        ];

        let pool_info = vk::DescriptorPoolCreateInfo::builder()
            .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
            .max_sets(1500)
            .pool_sizes(&pool_sizes);

        unsafe {
            device.create_descriptor_pool(&pool_info, None)
                .map_err(|e| VulkanError::PipelineCreation(e.to_string()))
        }
    }

    pub async fn render(&self, document: &Document, layout_engine: &LayoutEngine) -> Result<()> {
        let frame_start = std::time::Instant::now();
        
        let swapchain_data = self.swapchain_data.read();
        if swapchain_data.swapchain == vk::SwapchainKHR::null() {
            return Ok(());
        }
        
        let image_index = self.acquire_next_image(&swapchain_data)?;
        let command_buffer = self.command_manager.begin_frame().await?;
        
        self.begin_render_pass(command_buffer, &swapchain_data, image_index)?;
        
        let mut stats = RenderStats::default();
        {
            let mut batch = self.command_batches.write();
            batch.clear();
            
            self.build_render_commands(document, layout_engine, &mut batch).await?;
            stats.draw_calls = batch.len() as u32;
            
            for command in batch.iter() {
                self.execute_render_command(command_buffer, command, &mut stats)?;
            }
        }
        
        self.end_render_pass(command_buffer)?;
        self.command_manager.end_frame(command_buffer).await?;
        self.present_frame(&swapchain_data, image_index)?;
        
        stats.frame_time_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
        stats.memory_used_mb = (self.get_memory_usage().await / (1024 * 1024)) as u32;
        *self.stats.write() = stats;
        
        self.frame_index.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    async fn build_render_commands(
        &self, 
        _document: &Document, 
        _layout_engine: &LayoutEngine, 
        _batch: &mut Vec<RenderCommand>
    ) -> Result<()> {
        Ok(())
    }

    fn acquire_next_image(&self, swapchain_data: &SwapchainData) -> Result<u32> {
        let (image_index, _) = unsafe {
            self.swapchain_loader.acquire_next_image(
                swapchain_data.swapchain,
                u64::MAX,
                vk::Semaphore::null(),
                vk::Fence::null(),
            )
        }.map_err(|e| VulkanError::SwapchainCreation(e.to_string()))?;
        
        Ok(image_index)
    }

    fn begin_render_pass(
        &self, 
        command_buffer: vk::CommandBuffer, 
        swapchain_data: &SwapchainData, 
        image_index: u32
    ) -> Result<()> {
        let clear_values = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 1.0],
                },
            },
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 1.0,
                    stencil: 0,
                },
            },
        ];

        let render_pass_info = vk::RenderPassBeginInfo::builder()
            .render_pass(self.render_pass)
            .framebuffer(swapchain_data.framebuffers[image_index as usize])
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: swapchain_data.extent,
            })
            .clear_values(&clear_values);

        unsafe {
            self.device.logical_device().cmd_begin_render_pass(
                command_buffer,
                &render_pass_info,
                vk::SubpassContents::INLINE,
            );
        }

        Ok(())
    }

    fn execute_render_command(
        &self, 
        command_buffer: vk::CommandBuffer, 
        command: &RenderCommand, 
        stats: &mut RenderStats
    ) -> Result<()> {
        if let Some(pipeline) = self.resources.pipelines.get(&command.pipeline_id) {
            unsafe {
                self.device.logical_device().cmd_bind_pipeline(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    *pipeline.value(),
                );

                if !command.descriptor_sets.is_empty() {
                    self.device.logical_device().cmd_bind_descriptor_sets(
                        command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        vk::PipelineLayout::null(),
                        0,
                        &command.descriptor_sets,
                        &[],
                    );
                }

                self.device.logical_device().cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &[command.vertex_buffer],
                    &[command.vertex_offset as u64],
                );

                self.device.logical_device().cmd_bind_index_buffer(
                    command_buffer,
                    command.index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );

                self.device.logical_device().cmd_draw_indexed(
                    command_buffer,
                    command.index_count,
                    command.instance_count,
                    0,
                    0,
                    0,
                );
            }

            stats.triangles += command.index_count / 3;
            stats.vertices += command.index_count;
            stats.pipeline_switches += 1;
        }

        Ok(())
    }

    fn end_render_pass(&self, command_buffer: vk::CommandBuffer) -> Result<()> {
        unsafe {
            self.device.logical_device().cmd_end_render_pass(command_buffer);
        }
        Ok(())
    }

    fn present_frame(&self, swapchain_data: &SwapchainData, image_index: u32) -> Result<()> {
        let swapchains = [swapchain_data.swapchain];
        let image_indices = [image_index];

        let present_info = vk::PresentInfoKHR::builder()
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        unsafe {
            self.swapchain_loader.queue_present(self.device.graphics_queue(), &present_info)
                .map_err(|e| VulkanError::SwapchainCreation(e.to_string()))?;
        }

        Ok(())
    }

    pub async fn resize_surface(&self, width: u32, height: u32) -> Result<()> {
        self.device.wait_idle().await?;
        
        let mut swapchain_data = self.swapchain_data.write();
        swapchain_data.extent.width = width.max(1);
        swapchain_data.extent.height = height.max(1);
        
        Ok(())
    }

    pub async fn get_metrics(&self) -> serde_json::Value {
        let stats = self.stats.read();
        serde_json::json!({
            "frame_time_ms": stats.frame_time_ms,
            "fps": if stats.frame_time_ms > 0.0 { 1000.0 / stats.frame_time_ms } else { 0.0 },
            "draw_calls": stats.draw_calls,
            "triangles": stats.triangles,
            "vertices": stats.vertices,
            "memory_used_mb": stats.memory_used_mb,
            "pipeline_switches": stats.pipeline_switches,
            "frame_index": self.frame_index.load(std::sync::atomic::Ordering::Relaxed),
        })
    }

    pub async fn get_memory_usage(&self) -> u64 {
        self.memory_tracker.current_usage()
    }

    pub async fn get_peak_memory_usage(&self) -> u64 {
        self.memory_tracker.peak_usage()
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.device.wait_idle().await?;

        unsafe {
            self.resources.cleanup(self.device.logical_device());
            self.device.logical_device().destroy_descriptor_pool(self.descriptor_pool, None);
            self.device.logical_device().destroy_pipeline_cache(self.pipeline_cache, None);
            self.device.logical_device().destroy_render_pass(self.render_pass, None);
            
            if self.surface != vk::SurfaceKHR::null() {
                self.surface_loader.destroy_surface(self.surface, None);
            }
            
            self.instance.destroy_instance(None);
        }

        Ok(())
    }

    pub fn get_pipeline(&self, id: u64) -> Option<vk::Pipeline> {
        self.resources.pipelines.get(&id).map(|entry| *entry.value())
    }

    pub fn register_pipeline(&self, id: u64, pipeline: vk::Pipeline) {
        self.resources.pipelines.insert(id, pipeline);
    }
}