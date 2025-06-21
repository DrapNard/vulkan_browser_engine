use super::GpuError;
use ash::vk;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, Allocator};
use std::sync::Arc;

pub struct Buffer {
    device: Arc<ash::Device>,
    buffer: vk::Buffer,
    allocation: Option<Allocation>,
    size: u64,
    usage: vk::BufferUsageFlags,
}

impl Buffer {
    pub fn new(
        device: Arc<ash::Device>,
        allocator: Arc<Allocator>,
        size: u64,
        usage: vk::BufferUsageFlags,
        memory_location: gpu_allocator::MemoryLocation,
    ) -> Result<Self, GpuError> {
        let buffer_info = vk::BufferCreateInfo::builder()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        let buffer = unsafe {
            device
                .create_buffer(&buffer_info, None)
                .map_err(GpuError::VulkanError)?
        };

        let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };

        let allocation = allocator
            .allocate(&AllocationCreateDesc {
                name: "Buffer",
                requirements,
                location: memory_location,
                linear: true,
                allocation_scheme: gpu_allocator::vulkan::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(GpuError::AllocationError)?;

        unsafe {
            device
                .bind_buffer_memory(buffer, allocation.memory(), allocation.offset())
                .map_err(GpuError::VulkanError)?;
        }

        Ok(Self {
            device,
            buffer,
            allocation: Some(allocation),
            size,
            usage,
        })
    }

    pub fn write_data<T: Copy>(&mut self, data: &[T]) -> Result<(), GpuError> {
        if let Some(allocation) = &mut self.allocation {
            if let Some(mapped_ptr) = allocation.mapped_ptr() {
                let byte_size = std::mem::size_of_val(data);
                if byte_size > self.size as usize {
                    return Err(GpuError::BufferCreationFailed);
                }

                unsafe {
                    std::ptr::copy_nonoverlapping(
                        data.as_ptr() as *const u8,
                        mapped_ptr.as_ptr(),
                        byte_size,
                    );
                }
                Ok(())
            } else {
                Err(GpuError::MemoryMappingFailed)
            }
        } else {
            Err(GpuError::BufferCreationFailed)
        }
    }

    pub fn read_data<T: Copy>(&self, data: &mut [T]) -> Result<(), GpuError> {
        if let Some(allocation) = &self.allocation {
            if let Some(mapped_ptr) = allocation.mapped_ptr() {
                let byte_size = std::mem::size_of_val(data);
                if byte_size > self.size as usize {
                    return Err(GpuError::BufferCreationFailed);
                }

                unsafe {
                    std::ptr::copy_nonoverlapping(
                        mapped_ptr.as_ptr(),
                        data.as_mut_ptr() as *mut u8,
                        byte_size,
                    );
                }
                Ok(())
            } else {
                Err(GpuError::MemoryMappingFailed)
            }
        } else {
            Err(GpuError::BufferCreationFailed)
        }
    }

    pub fn get_buffer(&self) -> vk::Buffer {
        self.buffer
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn usage(&self) -> vk::BufferUsageFlags {
        self.usage
    }

    pub fn copy_to_buffer(&self, cmd: vk::CommandBuffer, dst_buffer: &Buffer, size: Option<u64>) -> Result<(), GpuError> {
        let copy_size = size.unwrap_or(self.size.min(dst_buffer.size));
        
        let copy_region = vk::BufferCopy::builder()
            .src_offset(0)
            .dst_offset(0)
            .size(copy_size);

        unsafe {
            self.device.cmd_copy_buffer(
                cmd,
                self.buffer,
                dst_buffer.buffer,
                &[copy_region.build()],
            );
        }

        Ok(())
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_buffer(self.buffer, None);
        }
    }
}

unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}