use super::GpuError;
use ash::vk;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, Allocator};
use std::sync::{Arc, Mutex};

pub struct Buffer {
    device: Arc<ash::Device>,
    buffer: vk::Buffer,
    allocation: Option<Allocation>,
    allocator: Arc<Mutex<Allocator>>,
    size: u64,
    usage: vk::BufferUsageFlags,
}

impl Buffer {
    pub fn new(
        device: Arc<ash::Device>,
        allocator: Arc<Mutex<Allocator>>,
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

        let allocation = {
            let mut allocator_guard = allocator.lock().unwrap();
            allocator_guard
                .allocate(&AllocationCreateDesc {
                    name: "Buffer",
                    requirements,
                    location: memory_location,
                    linear: true,
                    allocation_scheme: gpu_allocator::vulkan::AllocationScheme::GpuAllocatorManaged,
                })
                .map_err(GpuError::AllocationError)?
        };

        unsafe {
            device
                .bind_buffer_memory(buffer, allocation.memory(), allocation.offset())
                .map_err(GpuError::VulkanError)?;
        }

        Ok(Self {
            device,
            buffer,
            allocation: Some(allocation),
            allocator,
            size,
            usage,
        })
    }

    pub fn write_data<T: Copy>(&mut self, data: &[T]) -> Result<(), GpuError> {
        let byte_size = std::mem::size_of_val(data);
        
        if byte_size as u64 > self.size {
            return Err(GpuError::BufferCreationFailed);
        }

        if let Some(allocation) = &mut self.allocation {
            if let Some(mapped_ptr) = allocation.mapped_ptr() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        data.as_ptr() as *const u8,
                        mapped_ptr.as_ptr() as *mut u8,
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
        let byte_size = std::mem::size_of_val(data);
        
        if byte_size as u64 > self.size {
            return Err(GpuError::BufferCreationFailed);
        }

        if let Some(allocation) = &self.allocation {
            if let Some(mapped_ptr) = allocation.mapped_ptr() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        mapped_ptr.as_ptr() as *const u8,
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

    pub fn write_data_at_offset<T: Copy>(&mut self, data: &[T], offset: u64) -> Result<(), GpuError> {
        let byte_size = std::mem::size_of_val(data);
        
        if offset + byte_size as u64 > self.size {
            return Err(GpuError::BufferCreationFailed);
        }

        if let Some(allocation) = &mut self.allocation {
            if let Some(mapped_ptr) = allocation.mapped_ptr() {
                unsafe {
                    let dst_ptr = (mapped_ptr.as_ptr() as *mut u8).add(offset as usize);
                    std::ptr::copy_nonoverlapping(
                        data.as_ptr() as *const u8,
                        dst_ptr,
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

    pub fn flush_memory(&self) -> Result<(), GpuError> {
        if let Some(allocation) = &self.allocation {
            if allocation.mapped_ptr().is_some() {
                unsafe {
                    let memory_range = vk::MappedMemoryRange::builder()
                        .memory(allocation.memory())
                        .offset(allocation.offset())
                        .size(self.size);

                    self.device
                        .flush_mapped_memory_ranges(&[memory_range.build()])
                        .map_err(GpuError::VulkanError)?;
                }
            }
        }
        Ok(())
    }

    pub fn invalidate_memory(&self) -> Result<(), GpuError> {
        if let Some(allocation) = &self.allocation {
            if allocation.mapped_ptr().is_some() {
                unsafe {
                    let memory_range = vk::MappedMemoryRange::builder()
                        .memory(allocation.memory())
                        .offset(allocation.offset())
                        .size(self.size);

                    self.device
                        .invalidate_mapped_memory_ranges(&[memory_range.build()])
                        .map_err(GpuError::VulkanError)?;
                }
            }
        }
        Ok(())
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

    pub fn copy_to_buffer(
        &self,
        cmd: vk::CommandBuffer,
        dst_buffer: &Buffer,
        src_offset: u64,
        dst_offset: u64,
        size: Option<u64>,
    ) -> Result<(), GpuError> {
        let copy_size = size.unwrap_or_else(|| {
            (self.size - src_offset).min(dst_buffer.size - dst_offset)
        });
        
        if src_offset + copy_size > self.size {
            return Err(GpuError::BufferCreationFailed);
        }
        
        if dst_offset + copy_size > dst_buffer.size {
            return Err(GpuError::BufferCreationFailed);
        }
        
        let copy_region = vk::BufferCopy::builder()
            .src_offset(src_offset)
            .dst_offset(dst_offset)
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

    pub fn copy_from_buffer(
        &self,
        cmd: vk::CommandBuffer,
        src_buffer: &Buffer,
        src_offset: u64,
        dst_offset: u64,
        size: Option<u64>,
    ) -> Result<(), GpuError> {
        src_buffer.copy_to_buffer(cmd, self, src_offset, dst_offset, size)
    }

    pub fn clear(&mut self) -> Result<(), GpuError> {
        if let Some(allocation) = &mut self.allocation {
            if let Some(mapped_ptr) = allocation.mapped_ptr() {
                unsafe {
                    std::ptr::write_bytes(mapped_ptr.as_ptr() as *mut u8, 0, self.size as usize);
                }
                Ok(())
            } else {
                Err(GpuError::MemoryMappingFailed)
            }
        } else {
            Err(GpuError::BufferCreationFailed)
        }
    }

    pub fn is_mapped(&self) -> bool {
        self.allocation
            .as_ref()
            .map_or(false, |alloc| alloc.mapped_ptr().is_some())
    }

    pub fn get_device_address(&self) -> Result<vk::DeviceAddress, GpuError> {
        let buffer_device_address_info = vk::BufferDeviceAddressInfo::builder()
            .buffer(self.buffer);

        unsafe {
            Ok(self.device.get_buffer_device_address(&buffer_device_address_info))
        }
    }

    pub fn get_allocation_info(&self) -> Option<(vk::DeviceMemory, vk::DeviceSize, vk::DeviceSize)> {
        self.allocation.as_ref().map(|alloc| unsafe {
            (alloc.memory(), alloc.offset(), alloc.size())
        })
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        if let Some(allocation) = self.allocation.take() {
            let mut allocator_guard = self.allocator.lock().unwrap();
            let _ = allocator_guard.free(allocation);
        }
        
        unsafe {
            self.device.destroy_buffer(self.buffer, None);
        }
    }
}

unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}