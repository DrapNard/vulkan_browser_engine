pub mod buffer;
pub mod texture;

pub use buffer::*;
pub use texture::*;

use ash::vk;
use std::sync::Arc;

pub struct GpuContext {
    device: Arc<ash::Device>,
    memory_allocator: Arc<std::sync::Mutex<gpu_allocator::vulkan::Allocator>>,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    queue_family_index: u32,
}

impl GpuContext {
    pub fn new(
        device: Arc<ash::Device>,
        memory_allocator: Arc<std::sync::Mutex<gpu_allocator::vulkan::Allocator>>,
        command_pool: vk::CommandPool,
        queue: vk::Queue,
        queue_family_index: u32,
    ) -> Self {
        Self {
            device,
            memory_allocator,
            command_pool,
            queue,
            queue_family_index,
        }
    }

    pub fn create_buffer(
        &self,
        size: u64,
        usage: vk::BufferUsageFlags,
        memory_location: gpu_allocator::MemoryLocation,
    ) -> Result<Buffer, GpuError> {
        Buffer::new(
            self.device.clone(),
            self.memory_allocator.clone(),
            size,
            usage,
            memory_location,
        )
    }

    pub fn create_texture(
        &self,
        width: u32,
        height: u32,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
    ) -> Result<Texture, GpuError> {
        Texture::new(
            self.device.clone(),
            self.memory_allocator.clone(),
            width,
            height,
            format,
            usage,
        )
    }

    pub fn allocate_command_buffer(&self) -> Result<vk::CommandBuffer, GpuError> {
        let alloc_info = vk::CommandBufferAllocateInfo::builder()
            .command_pool(self.command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);

        unsafe {
            self.device
                .allocate_command_buffers(&alloc_info)
                .map_err(GpuError::VulkanError)?
                .into_iter()
                .next()
                .ok_or(GpuError::CommandBufferAllocationFailed)
        }
    }

    pub fn submit_command_buffer(
        &self,
        command_buffer: vk::CommandBuffer,
        wait_fence: Option<vk::Fence>,
    ) -> Result<(), GpuError> {
        let command_buffers = [command_buffer];
        let submit_info = vk::SubmitInfo::builder().command_buffers(&command_buffers);

        unsafe {
            self.device
                .queue_submit(
                    self.queue,
                    &[submit_info.build()],
                    wait_fence.unwrap_or(vk::Fence::null()),
                )
                .map_err(GpuError::VulkanError)
        }
    }

    pub fn wait_idle(&self) -> Result<(), GpuError> {
        unsafe {
            self.device
                .device_wait_idle()
                .map_err(GpuError::VulkanError)
        }
    }

    pub fn get_device(&self) -> &ash::Device {
        &self.device
    }

    pub fn get_queue(&self) -> vk::Queue {
        self.queue
    }

    pub fn get_queue_family_index(&self) -> u32 {
        self.queue_family_index
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    #[error("Vulkan error: {0}")]
    VulkanError(vk::Result),
    #[error("Allocation error: {0}")]
    AllocationError(#[from] gpu_allocator::AllocationError),
    #[error("Command buffer allocation failed")]
    CommandBufferAllocationFailed,
    #[error("Buffer creation failed")]
    BufferCreationFailed,
    #[error("Texture creation failed")]
    TextureCreationFailed,
    #[error("Memory mapping failed")]
    MemoryMappingFailed,
}
