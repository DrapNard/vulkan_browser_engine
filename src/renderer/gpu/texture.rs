use super::GpuError;
use ash::vk;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, Allocator};
use std::sync::{Arc, Mutex};

pub struct Texture {
    device: Arc<ash::Device>,
    image: vk::Image,
    image_view: vk::ImageView,
    sampler: vk::Sampler,
    allocation: Option<Allocation>,
    allocator: Arc<Mutex<Allocator>>,
    width: u32,
    height: u32,
    format: vk::Format,
    mip_levels: u32,
}

impl Texture {
    pub fn new(
        device: Arc<ash::Device>,
        allocator: Arc<Mutex<Allocator>>,
        width: u32,
        height: u32,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
    ) -> Result<Self, GpuError> {
        Self::new_with_mips(device, allocator, width, height, format, usage, None)
    }

    pub fn new_with_mips(
        device: Arc<ash::Device>,
        allocator: Arc<Mutex<Allocator>>,
        width: u32,
        height: u32,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        custom_mip_levels: Option<u32>,
    ) -> Result<Self, GpuError> {
        if width == 0 || height == 0 {
            return Err(GpuError::TextureCreationFailed);
        }

        let mip_levels = custom_mip_levels
            .unwrap_or_else(|| ((width.max(height) as f32).log2().floor() as u32) + 1)
            .max(1);

        let image_info = vk::ImageCreateInfo::builder()
            .image_type(vk::ImageType::TYPE_2D)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(mip_levels)
            .array_layers(1)
            .format(format)
            .tiling(vk::ImageTiling::OPTIMAL)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .samples(vk::SampleCountFlags::TYPE_1);

        let image = unsafe {
            device
                .create_image(&image_info, None)
                .map_err(GpuError::VulkanError)?
        };

        let requirements = unsafe { device.get_image_memory_requirements(image) };

        let allocation = {
            let mut allocator_guard = allocator.lock().unwrap();
            allocator_guard
                .allocate(&AllocationCreateDesc {
                    name: "Texture",
                    requirements,
                    location: gpu_allocator::MemoryLocation::GpuOnly,
                    linear: false,
                    allocation_scheme: gpu_allocator::vulkan::AllocationScheme::GpuAllocatorManaged,
                })
                .map_err(GpuError::AllocationError)?
        };

        unsafe {
            device
                .bind_image_memory(image, allocation.memory(), allocation.offset())
                .map_err(|e| {
                    device.destroy_image(image, None);
                    GpuError::VulkanError(e)
                })?;
        }

        let image_view = Self::create_image_view(&device, image, format, mip_levels).inspect_err(|_e| {
            unsafe { device.destroy_image(image, None) };
        })?;

        let sampler = Self::create_sampler(&device, mip_levels).inspect_err(|_e| {
            unsafe { 
                device.destroy_image_view(image_view, None);
                device.destroy_image(image, None);
            };
        })?;

        Ok(Self {
            device,
            image,
            image_view,
            sampler,
            allocation: Some(allocation),
            allocator,
            width,
            height,
            format,
            mip_levels,
        })
    }

    pub fn new_depth_texture(
        device: Arc<ash::Device>,
        allocator: Arc<Mutex<Allocator>>,
        width: u32,
        height: u32,
        format: vk::Format,
    ) -> Result<Self, GpuError> {
        if !Self::is_depth_format(format) {
            return Err(GpuError::TextureCreationFailed);
        }

        let usage = vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED;
        let mut texture = Self::new_with_mips(device, allocator, width, height, format, usage, Some(1))?;

        unsafe {
            texture.device.destroy_image_view(texture.image_view, None);
        }

        texture.image_view = Self::create_depth_image_view(&texture.device, texture.image, format)?;

        Ok(texture)
    }

    fn create_image_view(
        device: &ash::Device,
        image: vk::Image,
        format: vk::Format,
        mip_levels: u32,
    ) -> Result<vk::ImageView, GpuError> {
        let view_info = vk::ImageViewCreateInfo::builder()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(
                vk::ImageSubresourceRange::builder()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(mip_levels)
                    .base_array_layer(0)
                    .layer_count(1)
                    .build(),
            );

        unsafe {
            device
                .create_image_view(&view_info, None)
                .map_err(GpuError::VulkanError)
        }
    }

    fn create_depth_image_view(
        device: &ash::Device,
        image: vk::Image,
        format: vk::Format,
    ) -> Result<vk::ImageView, GpuError> {
        let aspect_mask = if format == vk::Format::D32_SFLOAT_S8_UINT || format == vk::Format::D24_UNORM_S8_UINT {
            vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL
        } else {
            vk::ImageAspectFlags::DEPTH
        };

        let view_info = vk::ImageViewCreateInfo::builder()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(
                vk::ImageSubresourceRange::builder()
                    .aspect_mask(aspect_mask)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1)
                    .build(),
            );

        unsafe {
            device
                .create_image_view(&view_info, None)
                .map_err(GpuError::VulkanError)
        }
    }

    fn create_sampler(device: &ash::Device, mip_levels: u32) -> Result<vk::Sampler, GpuError> {
        let sampler_info = vk::SamplerCreateInfo::builder()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::REPEAT)
            .address_mode_v(vk::SamplerAddressMode::REPEAT)
            .address_mode_w(vk::SamplerAddressMode::REPEAT)
            .anisotropy_enable(true)
            .max_anisotropy(16.0)
            .border_color(vk::BorderColor::INT_OPAQUE_BLACK)
            .unnormalized_coordinates(false)
            .compare_enable(false)
            .compare_op(vk::CompareOp::ALWAYS)
            .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
            .mip_lod_bias(0.0)
            .min_lod(0.0)
            .max_lod(mip_levels as f32);

        unsafe {
            device
                .create_sampler(&sampler_info, None)
                .map_err(GpuError::VulkanError)
        }
    }

    pub fn create_nearest_sampler(device: &ash::Device) -> Result<vk::Sampler, GpuError> {
        let sampler_info = vk::SamplerCreateInfo::builder()
            .mag_filter(vk::Filter::NEAREST)
            .min_filter(vk::Filter::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .anisotropy_enable(false)
            .max_anisotropy(1.0)
            .border_color(vk::BorderColor::INT_OPAQUE_BLACK)
            .unnormalized_coordinates(false)
            .compare_enable(false)
            .compare_op(vk::CompareOp::ALWAYS)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .mip_lod_bias(0.0)
            .min_lod(0.0)
            .max_lod(0.0);

        unsafe {
            device
                .create_sampler(&sampler_info, None)
                .map_err(GpuError::VulkanError)
        }
    }

    pub fn transition_layout(
        &self,
        cmd: vk::CommandBuffer,
        old_layout: vk::ImageLayout,
        new_layout: vk::ImageLayout,
    ) -> Result<(), GpuError> {
        self.transition_layout_with_range(cmd, old_layout, new_layout, 0, self.mip_levels)
    }

    pub fn transition_layout_with_range(
        &self,
        cmd: vk::CommandBuffer,
        old_layout: vk::ImageLayout,
        new_layout: vk::ImageLayout,
        base_mip: u32,
        mip_count: u32,
    ) -> Result<(), GpuError> {
        if base_mip + mip_count > self.mip_levels {
            return Err(GpuError::TextureCreationFailed);
        }

        let aspect_mask = if Self::is_depth_format(self.format) {
            let mut mask = vk::ImageAspectFlags::DEPTH;
            if self.format == vk::Format::D32_SFLOAT_S8_UINT || self.format == vk::Format::D24_UNORM_S8_UINT {
                mask |= vk::ImageAspectFlags::STENCIL;
            }
            mask
        } else {
            vk::ImageAspectFlags::COLOR
        };

        let barrier = vk::ImageMemoryBarrier::builder()
            .old_layout(old_layout)
            .new_layout(new_layout)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.image)
            .subresource_range(
                vk::ImageSubresourceRange::builder()
                    .aspect_mask(aspect_mask)
                    .base_mip_level(base_mip)
                    .level_count(mip_count)
                    .base_array_layer(0)
                    .layer_count(1)
                    .build(),
            );

        let (src_stage, dst_stage, src_access, dst_access) = match (old_layout, new_layout) {
            (vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL) => (
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::AccessFlags::empty(),
                vk::AccessFlags::TRANSFER_WRITE,
            ),
            (vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL) => (
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::AccessFlags::TRANSFER_WRITE,
                vk::AccessFlags::SHADER_READ,
            ),
            (vk::ImageLayout::UNDEFINED, vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL) => (
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
                vk::AccessFlags::empty(),
                vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            ),
            (vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::TRANSFER_SRC_OPTIMAL) => (
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::TRANSFER,
                vk::AccessFlags::TRANSFER_WRITE,
                vk::AccessFlags::TRANSFER_READ,
            ),
            _ => {
                return Err(GpuError::TextureCreationFailed);
            }
        };

        let barrier = barrier
            .src_access_mask(src_access)
            .dst_access_mask(dst_access);

        unsafe {
            self.device.cmd_pipeline_barrier(
                cmd,
                src_stage,
                dst_stage,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier.build()],
            );
        }

        Ok(())
    }

    pub fn copy_from_buffer(&self, cmd: vk::CommandBuffer, buffer: vk::Buffer) -> Result<(), GpuError> {
        self.copy_from_buffer_with_offset(cmd, buffer, 0, 0)
    }

    pub fn copy_from_buffer_with_offset(
        &self,
        cmd: vk::CommandBuffer,
        buffer: vk::Buffer,
        buffer_offset: u64,
        mip_level: u32,
    ) -> Result<(), GpuError> {
        if mip_level >= self.mip_levels {
            return Err(GpuError::TextureCreationFailed);
        }

        let mip_width = (self.width >> mip_level).max(1);
        let mip_height = (self.height >> mip_level).max(1);

        let region = vk::BufferImageCopy::builder()
            .buffer_offset(buffer_offset)
            .buffer_row_length(0)
            .buffer_image_height(0)
            .image_subresource(
                vk::ImageSubresourceLayers::builder()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(mip_level)
                    .base_array_layer(0)
                    .layer_count(1)
                    .build(),
            )
            .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
            .image_extent(vk::Extent3D {
                width: mip_width,
                height: mip_height,
                depth: 1,
            });

        unsafe {
            self.device.cmd_copy_buffer_to_image(
                cmd,
                buffer,
                self.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region.build()],
            );
        }

        Ok(())
    }

    pub fn generate_mipmaps(&self, cmd: vk::CommandBuffer) -> Result<(), GpuError> {
        if self.mip_levels <= 1 {
            return Ok(());
        }

        let mut mip_width = self.width as i32;
        let mut mip_height = self.height as i32;

        for i in 1..self.mip_levels {
            let barrier = vk::ImageMemoryBarrier::builder()
                .image(self.image)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .subresource_range(
                    vk::ImageSubresourceRange::builder()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .base_mip_level(i - 1)
                        .level_count(1)
                        .base_array_layer(0)
                        .layer_count(1)
                        .build(),
                )
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(vk::AccessFlags::TRANSFER_READ);

            unsafe {
                self.device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier.build()],
                );
            }

            let next_mip_width = if mip_width > 1 { mip_width / 2 } else { 1 };
            let next_mip_height = if mip_height > 1 { mip_height / 2 } else { 1 };

            let blit = vk::ImageBlit::builder()
                .src_offsets([
                    vk::Offset3D { x: 0, y: 0, z: 0 },
                    vk::Offset3D {
                        x: mip_width,
                        y: mip_height,
                        z: 1,
                    },
                ])
                .src_subresource(
                    vk::ImageSubresourceLayers::builder()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(i - 1)
                        .base_array_layer(0)
                        .layer_count(1)
                        .build(),
                )
                .dst_offsets([
                    vk::Offset3D { x: 0, y: 0, z: 0 },
                    vk::Offset3D {
                        x: next_mip_width,
                        y: next_mip_height,
                        z: 1,
                    },
                ])
                .dst_subresource(
                    vk::ImageSubresourceLayers::builder()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(i)
                        .base_array_layer(0)
                        .layer_count(1)
                        .build(),
                );

            unsafe {
                self.device.cmd_blit_image(
                    cmd,
                    self.image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    self.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[blit.build()],
                    vk::Filter::LINEAR,
                );
            }

            let final_barrier = vk::ImageMemoryBarrier::builder()
                .image(self.image)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .subresource_range(
                    vk::ImageSubresourceRange::builder()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .base_mip_level(i - 1)
                        .level_count(1)
                        .base_array_layer(0)
                        .layer_count(1)
                        .build(),
                )
                .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                .dst_access_mask(vk::AccessFlags::SHADER_READ);

            unsafe {
                self.device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[final_barrier.build()],
                );
            }

            mip_width = next_mip_width;
            mip_height = next_mip_height;
        }

        let last_mip_barrier = vk::ImageMemoryBarrier::builder()
            .image(self.image)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .subresource_range(
                vk::ImageSubresourceRange::builder()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(self.mip_levels - 1)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1)
                    .build(),
            )
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ);

        unsafe {
            self.device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[last_mip_barrier.build()],
            );
        }

        Ok(())
    }

    pub fn get_image(&self) -> vk::Image {
        self.image
    }

    pub fn get_image_view(&self) -> vk::ImageView {
        self.image_view
    }

    pub fn get_sampler(&self) -> vk::Sampler {
        self.sampler
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn format(&self) -> vk::Format {
        self.format
    }

    pub fn mip_levels(&self) -> u32 {
        self.mip_levels
    }

    pub fn get_extent(&self) -> vk::Extent2D {
        vk::Extent2D {
            width: self.width,
            height: self.height,
        }
    }

    pub fn get_mip_extent(&self, mip_level: u32) -> Result<vk::Extent2D, GpuError> {
        if mip_level >= self.mip_levels {
            return Err(GpuError::TextureCreationFailed);
        }

        Ok(vk::Extent2D {
            width: (self.width >> mip_level).max(1),
            height: (self.height >> mip_level).max(1),
        })
    }

    pub fn get_allocation_info(&self) -> Option<(vk::DeviceMemory, vk::DeviceSize, vk::DeviceSize)> {
        self.allocation.as_ref().map(|alloc| unsafe {
            (alloc.memory(), alloc.offset(), alloc.size())
        })
    }

    pub fn is_depth_texture(&self) -> bool {
        Self::is_depth_format(self.format)
    }

    fn is_depth_format(format: vk::Format) -> bool {
        matches!(format, 
            vk::Format::D16_UNORM |
            vk::Format::D32_SFLOAT |
            vk::Format::D16_UNORM_S8_UINT |
            vk::Format::D24_UNORM_S8_UINT |
            vk::Format::D32_SFLOAT_S8_UINT
        )
    }

    pub fn calculate_required_bytes(&self) -> u64 {
        let mut total_bytes = 0u64;
        for mip in 0..self.mip_levels {
            let mip_width = (self.width >> mip).max(1) as u64;
            let mip_height = (self.height >> mip).max(1) as u64;
            let bytes_per_pixel = Self::get_format_bytes_per_pixel(self.format);
            total_bytes += mip_width * mip_height * bytes_per_pixel as u64;
        }
        total_bytes
    }

    fn get_format_bytes_per_pixel(format: vk::Format) -> u32 {
        match format {
            vk::Format::R8_UNORM | vk::Format::R8_UINT | vk::Format::R8_SINT => 1,
            vk::Format::R8G8_UNORM | vk::Format::R8G8_UINT | vk::Format::R8G8_SINT => 2,
            vk::Format::R8G8B8_UNORM | vk::Format::R8G8B8_UINT | vk::Format::R8G8B8_SINT => 3,
            vk::Format::R8G8B8A8_UNORM | vk::Format::R8G8B8A8_UINT | vk::Format::R8G8B8A8_SINT => 4,
            vk::Format::R16_UNORM | vk::Format::R16_UINT | vk::Format::R16_SINT | vk::Format::R16_SFLOAT => 2,
            vk::Format::R16G16_UNORM | vk::Format::R16G16_UINT | vk::Format::R16G16_SINT | vk::Format::R16G16_SFLOAT => 4,
            vk::Format::R16G16B16_UNORM | vk::Format::R16G16B16_UINT | vk::Format::R16G16B16_SINT | vk::Format::R16G16B16_SFLOAT => 6,
            vk::Format::R16G16B16A16_UNORM | vk::Format::R16G16B16A16_UINT | vk::Format::R16G16B16A16_SINT | vk::Format::R16G16B16A16_SFLOAT => 8,
            vk::Format::R32_UINT | vk::Format::R32_SINT | vk::Format::R32_SFLOAT => 4,
            vk::Format::R32G32_UINT | vk::Format::R32G32_SINT | vk::Format::R32G32_SFLOAT => 8,
            vk::Format::R32G32B32_UINT | vk::Format::R32G32B32_SINT | vk::Format::R32G32B32_SFLOAT => 12,
            vk::Format::R32G32B32A32_UINT | vk::Format::R32G32B32A32_SINT | vk::Format::R32G32B32A32_SFLOAT => 16,
            vk::Format::D16_UNORM => 2,
            vk::Format::D32_SFLOAT => 4,
            vk::Format::D24_UNORM_S8_UINT | vk::Format::D32_SFLOAT_S8_UINT => 4,
            _ => 4,
        }
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        if let Some(allocation) = self.allocation.take() {
            let mut allocator_guard = self.allocator.lock().unwrap();
            let _ = allocator_guard.free(allocation);
        }

        unsafe {
            self.device.destroy_sampler(self.sampler, None);
            self.device.destroy_image_view(self.image_view, None);
            self.device.destroy_image(self.image, None);
        }
    }
}

unsafe impl Send for Texture {}
unsafe impl Sync for Texture {}