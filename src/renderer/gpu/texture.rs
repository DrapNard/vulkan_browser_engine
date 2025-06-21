use super::GpuError;
use ash::vk;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, Allocator};
use std::sync::Arc;

pub struct Texture {
    device: Arc<ash::Device>,
    image: vk::Image,
    image_view: vk::ImageView,
    sampler: vk::Sampler,
    allocation: Option<Allocation>,
    width: u32,
    height: u32,
    format: vk::Format,
    mip_levels: u32,
}

impl Texture {
    pub fn new(
        device: Arc<ash::Device>,
        allocator: Arc<Allocator>,
        width: u32,
        height: u32,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
    ) -> Result<Self, GpuError> {
        let mip_levels = ((width.max(height) as f32).log2().floor() as u32) + 1;

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

        let allocation = allocator
            .allocate(&AllocationCreateDesc {
                name: "Texture",
                requirements,
                location: gpu_allocator::MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: gpu_allocator::vulkan::AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(GpuError::AllocationError)?;

        unsafe {
            device
                .bind_image_memory(image, allocation.memory(), allocation.offset())
                .map_err(GpuError::VulkanError)?;
        }

        let image_view = Self::create_image_view(&device, image, format, mip_levels)?;
        let sampler = Self::create_sampler(&device, mip_levels)?;

        Ok(Self {
            device,
            image,
            image_view,
            sampler,
            allocation: Some(allocation),
            width,
            height,
            format,
            mip_levels,
        })
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

    pub fn transition_layout(
        &self,
        cmd: vk::CommandBuffer,
        old_layout: vk::ImageLayout,
        new_layout: vk::ImageLayout,
    ) -> Result<(), GpuError> {
        let barrier = vk::ImageMemoryBarrier::builder()
            .old_layout(old_layout)
            .new_layout(new_layout)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.image)
            .subresource_range(
                vk::ImageSubresourceRange::builder()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(self.mip_levels)
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
        let region = vk::BufferImageCopy::builder()
            .buffer_offset(0)
            .buffer_row_length(0)
            .buffer_image_height(0)
            .image_subresource(
                vk::ImageSubresourceLayers::builder()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(0)
                    .base_array_layer(0)
                    .layer_count(1)
                    .build(),
            )
            .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
            .image_extent(vk::Extent3D {
                width: self.width,
                height: self.height,
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
                        x: if mip_width > 1 { mip_width / 2 } else { 1 },
                        y: if mip_height > 1 { mip_height / 2 } else { 1 },
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

            if mip_width > 1 {
                mip_width /= 2;
            }
            if mip_height > 1 {
                mip_height /= 2;
            }
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
}

impl Drop for Texture {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_sampler(self.sampler, None);
            self.device.destroy_image_view(self.image_view, None);
            self.device.destroy_image(self.image, None);
        }
    }
}

impl TextureManager {
    pub fn create_texture(&self, width: u32, height: u32, format: vk::Format) -> Result<Texture> {
        let image_info = vk::ImageCreateInfo::builder()
            .image_type(vk::ImageType::TYPE_2D)
            .extent(vk::Extent3D { width, height, depth: 1 })
            .mip_levels(1)
            .array_layers(1)
            .format(format)
            .tiling(vk::ImageTiling::OPTIMAL)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        let image = unsafe { self.device.create_image(&image_info, None)? };
        let requirements = unsafe { self.device.get_image_memory_requirements(image) };

        let allocation = self.allocator.lock().unwrap()
            .allocate(&gpu_allocator::vulkan::AllocationCreateDesc {
                name: "Texture",
                requirements,
                location: gpu_allocator::MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: gpu_allocator::vulkan::AllocationScheme::GpuAllocatorManaged,
            })?;

        unsafe {
            self.device.bind_image_memory(image, allocation.memory(), allocation.offset())?;
        }

        Ok(Texture { image, allocation: Some(allocation) })
    }
}

unsafe impl Send for Texture {}
unsafe impl Sync for Texture {}