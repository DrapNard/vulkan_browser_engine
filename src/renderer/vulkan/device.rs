use ash::extensions::khr::Surface;
use ash::vk;
use ash::{Device, Entry, Instance};
use parking_lot::RwLock;
use smallvec::SmallVec;
use std::collections::HashSet;
use std::ffi::CStr;
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DeviceError {
    #[error("No suitable physical device found")]
    NoSuitableDevice,
    #[error("Queue family not found: {0}")]
    QueueFamilyNotFound(String),
    #[error("Device creation failed: {0}")]
    DeviceCreation(String),
    #[error("Extension not supported: {0}")]
    ExtensionNotSupported(String),
    #[error("Feature not supported: {0}")]
    FeatureNotSupported(String),
}

pub type Result<T> = std::result::Result<T, DeviceError>;

#[derive(Debug, Clone, Copy)]
pub struct QueueFamilyIndices {
    pub graphics: u32,
    pub compute: u32,
    pub transfer: u32,
    pub present: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct DeviceCapabilities {
    pub max_memory_allocation_count: u32,
    pub max_bound_descriptor_sets: u32,
    pub max_push_constants_size: u32,
    pub max_uniform_buffer_range: u32,
    pub max_storage_buffer_range: u32,
    pub max_vertex_input_attributes: u32,
    pub max_vertex_input_bindings: u32,
    pub max_fragment_output_attachments: u32,
    pub max_compute_work_group_size: [u32; 3],
    pub max_compute_work_group_invocations: u32,
    pub supports_ray_tracing: bool,
    pub supports_mesh_shaders: bool,
    pub supports_variable_rate_shading: bool,
    pub supports_timeline_semaphores: bool,
}

pub struct VulkanDevice {
    physical_device: vk::PhysicalDevice,
    logical_device: Device,
    graphics_queue: vk::Queue,
    compute_queue: vk::Queue,
    transfer_queue: vk::Queue,
    present_queue: Option<vk::Queue>,
    queue_families: QueueFamilyIndices,
    capabilities: DeviceCapabilities,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
    device_properties: vk::PhysicalDeviceProperties,
    enabled_extensions: Arc<RwLock<HashSet<String>>>,
}

impl VulkanDevice {
    pub async fn new(
        entry: &Entry,
        instance: &Instance,
        surface: vk::SurfaceKHR,
        surface_loader: &Surface,
    ) -> Result<Self> {
        let physical_device =
            Self::select_physical_device(entry, instance, surface, surface_loader)?;

        let queue_families =
            Self::find_queue_families(instance, physical_device, surface, surface_loader)?;

        let capabilities = Self::query_device_capabilities(instance, physical_device)?;

        let (logical_device, queues) =
            Self::create_logical_device(instance, physical_device, queue_families, &capabilities)?;

        let memory_properties =
            unsafe { instance.get_physical_device_memory_properties(physical_device) };

        let device_properties = unsafe { instance.get_physical_device_properties(physical_device) };

        Ok(Self {
            physical_device,
            logical_device,
            graphics_queue: queues.0,
            compute_queue: queues.1,
            transfer_queue: queues.2,
            present_queue: queues.3,
            queue_families,
            capabilities,
            memory_properties,
            device_properties,
            enabled_extensions: Arc::new(RwLock::new(HashSet::new())),
        })
    }

    fn check_device_suitability(
        _entry: &Entry,
        instance: &Instance,
        device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
        surface_loader: &Surface,
    ) -> Result<bool> {
        let _queue_families = Self::find_queue_families(instance, device, surface, surface_loader)?;
        let _features = unsafe { instance.get_physical_device_features(device) };

        Ok(true)
    }

    fn select_physical_device(
        entry: &Entry,
        instance: &Instance,
        surface: vk::SurfaceKHR,
        surface_loader: &Surface,
    ) -> Result<vk::PhysicalDevice> {
        let physical_devices = unsafe {
            instance
                .enumerate_physical_devices()
                .map_err(|e| DeviceError::DeviceCreation(e.to_string()))?
        };

        if physical_devices.is_empty() {
            return Err(DeviceError::NoSuitableDevice);
        }

        let mut best_device = None;
        let mut best_score = 0u32;

        for device in physical_devices {
            let score =
                Self::rate_device_suitability(entry, instance, device, surface, surface_loader)?;
            if score > best_score {
                best_score = score;
                best_device = Some(device);
            }
        }

        best_device.ok_or(DeviceError::NoSuitableDevice)
    }

    fn rate_device_suitability(
        _entry: &Entry,
        instance: &Instance,
        device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
        surface_loader: &Surface,
    ) -> Result<u32> {
        let properties = unsafe { instance.get_physical_device_properties(device) };
        let features = unsafe { instance.get_physical_device_features(device) };

        let mut score = 0u32;

        match properties.device_type {
            vk::PhysicalDeviceType::DISCRETE_GPU => score += 1000,
            vk::PhysicalDeviceType::INTEGRATED_GPU => score += 500,
            _ => score += 100,
        }

        score += properties.limits.max_image_dimension2_d;

        if features.geometry_shader == vk::TRUE {
            score += 100;
        }

        if features.tessellation_shader == vk::TRUE {
            score += 100;
        }

        if features.multi_viewport == vk::TRUE {
            score += 50;
        }

        let _queue_families = Self::find_queue_families(instance, device, surface, surface_loader)?;

        let extensions = Self::get_required_extensions();
        let supported_extensions = Self::get_supported_extensions(instance, device)?;

        for ext in &extensions {
            let ext_str = ext.to_str().unwrap();
            if !supported_extensions.contains(ext_str) {
                return Ok(0);
            }
        }

        Ok(score)
    }

    fn find_queue_families(
        instance: &Instance,
        device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
        surface_loader: &Surface,
    ) -> Result<QueueFamilyIndices> {
        let queue_families =
            unsafe { instance.get_physical_device_queue_family_properties(device) };

        let mut graphics_family = None;
        let mut compute_family = None;
        let mut transfer_family = None;
        let mut present_family = None;

        for (index, family) in queue_families.iter().enumerate() {
            if family.queue_flags.contains(vk::QueueFlags::GRAPHICS) && graphics_family.is_none() {
                graphics_family = Some(index as u32);
            }

            if family.queue_flags.contains(vk::QueueFlags::COMPUTE) && compute_family.is_none() {
                compute_family = Some(index as u32);
            }

            if family.queue_flags.contains(vk::QueueFlags::TRANSFER) && transfer_family.is_none() {
                transfer_family = Some(index as u32);
            }

            if surface != vk::SurfaceKHR::null() {
                let supports_present = unsafe {
                    surface_loader
                        .get_physical_device_surface_support(device, index as u32, surface)
                        .unwrap_or(false)
                };

                if supports_present && present_family.is_none() {
                    present_family = Some(index as u32);
                }
            }
        }

        Ok(QueueFamilyIndices {
            graphics: graphics_family
                .ok_or(DeviceError::QueueFamilyNotFound("graphics".to_string()))?,
            compute: compute_family
                .ok_or(DeviceError::QueueFamilyNotFound("compute".to_string()))?,
            transfer: transfer_family
                .ok_or(DeviceError::QueueFamilyNotFound("transfer".to_string()))?,
            present: present_family,
        })
    }

    fn query_device_capabilities(
        instance: &Instance,
        device: vk::PhysicalDevice,
    ) -> Result<DeviceCapabilities> {
        let properties = unsafe { instance.get_physical_device_properties(device) };
        let _features = unsafe { instance.get_physical_device_features(device) };

        let mut features11 = vk::PhysicalDeviceVulkan11Features::default();
        let mut features12 = vk::PhysicalDeviceVulkan12Features::default();
        let mut features13 = vk::PhysicalDeviceVulkan13Features::default();

        let mut features2 = vk::PhysicalDeviceFeatures2::builder()
            .push_next(&mut features11)
            .push_next(&mut features12)
            .push_next(&mut features13)
            .build();

        unsafe {
            instance.get_physical_device_features2(device, &mut features2);
        }

        Ok(DeviceCapabilities {
            max_memory_allocation_count: properties.limits.max_memory_allocation_count,
            max_bound_descriptor_sets: properties.limits.max_bound_descriptor_sets,
            max_push_constants_size: properties.limits.max_push_constants_size,
            max_uniform_buffer_range: properties.limits.max_uniform_buffer_range,
            max_storage_buffer_range: properties.limits.max_storage_buffer_range,
            max_vertex_input_attributes: properties.limits.max_vertex_input_attributes,
            max_vertex_input_bindings: properties.limits.max_vertex_input_bindings,
            max_fragment_output_attachments: properties.limits.max_fragment_output_attachments,
            max_compute_work_group_size: properties.limits.max_compute_work_group_size,
            max_compute_work_group_invocations: properties
                .limits
                .max_compute_work_group_invocations,
            supports_ray_tracing: false,
            supports_mesh_shaders: false,
            supports_variable_rate_shading: false,
            supports_timeline_semaphores: features12.timeline_semaphore == vk::TRUE,
        })
    }

    fn create_logical_device(
        instance: &Instance,
        physical_device: vk::PhysicalDevice,
        queue_families: QueueFamilyIndices,
        capabilities: &DeviceCapabilities,
    ) -> Result<(Device, (vk::Queue, vk::Queue, vk::Queue, Option<vk::Queue>))> {
        let queue_priorities = [1.0];

        let mut queue_create_infos = SmallVec::<[vk::DeviceQueueCreateInfo; 4]>::new();
        let mut unique_queue_families = HashSet::new();

        unique_queue_families.insert(queue_families.graphics);
        unique_queue_families.insert(queue_families.compute);
        unique_queue_families.insert(queue_families.transfer);

        if let Some(present) = queue_families.present {
            unique_queue_families.insert(present);
        }

        for &queue_family in &unique_queue_families {
            queue_create_infos.push(
                vk::DeviceQueueCreateInfo::builder()
                    .queue_family_index(queue_family)
                    .queue_priorities(&queue_priorities)
                    .build(),
            );
        }

        let device_features = vk::PhysicalDeviceFeatures::builder()
            .geometry_shader(true)
            .tessellation_shader(true)
            .fill_mode_non_solid(true)
            .wide_lines(true)
            .large_points(true)
            .multi_viewport(true)
            .sampler_anisotropy(true)
            .texture_compression_bc(true)
            .texture_compression_etc2(true)
            .texture_compression_astc_ldr(true)
            .vertex_pipeline_stores_and_atomics(true)
            .fragment_stores_and_atomics(true)
            .shader_storage_image_extended_formats(true)
            .shader_uniform_buffer_array_dynamic_indexing(true)
            .shader_sampled_image_array_dynamic_indexing(true)
            .shader_storage_buffer_array_dynamic_indexing(true)
            .shader_storage_image_array_dynamic_indexing(true);

        let mut features11 = vk::PhysicalDeviceVulkan11Features::builder()
            .storage_buffer16_bit_access(true)
            .uniform_and_storage_buffer16_bit_access(true)
            .storage_push_constant16(true)
            .storage_input_output16(true)
            .multiview(true)
            .multiview_geometry_shader(true)
            .multiview_tessellation_shader(true)
            .variable_pointers_storage_buffer(true)
            .variable_pointers(true)
            .protected_memory(false);

        let mut features12 = vk::PhysicalDeviceVulkan12Features::builder()
            .sampler_mirror_clamp_to_edge(true)
            .draw_indirect_count(true)
            .storage_buffer8_bit_access(true)
            .uniform_and_storage_buffer8_bit_access(true)
            .storage_push_constant8(true)
            .shader_buffer_int64_atomics(true)
            .shader_shared_int64_atomics(true)
            .shader_float16(true)
            .shader_int8(true)
            .descriptor_indexing(true)
            .shader_input_attachment_array_dynamic_indexing(true)
            .shader_uniform_texel_buffer_array_dynamic_indexing(true)
            .shader_storage_texel_buffer_array_dynamic_indexing(true)
            .shader_uniform_buffer_array_non_uniform_indexing(true)
            .shader_sampled_image_array_non_uniform_indexing(true)
            .shader_storage_buffer_array_non_uniform_indexing(true)
            .shader_storage_image_array_non_uniform_indexing(true)
            .shader_input_attachment_array_non_uniform_indexing(true)
            .shader_uniform_texel_buffer_array_non_uniform_indexing(true)
            .shader_storage_texel_buffer_array_non_uniform_indexing(true)
            .descriptor_binding_uniform_buffer_update_after_bind(true)
            .descriptor_binding_sampled_image_update_after_bind(true)
            .descriptor_binding_storage_image_update_after_bind(true)
            .descriptor_binding_storage_buffer_update_after_bind(true)
            .descriptor_binding_uniform_texel_buffer_update_after_bind(true)
            .descriptor_binding_storage_texel_buffer_update_after_bind(true)
            .descriptor_binding_update_unused_while_pending(true)
            .descriptor_binding_partially_bound(true)
            .descriptor_binding_variable_descriptor_count(true)
            .runtime_descriptor_array(true)
            .sampler_filter_minmax(true)
            .scalar_block_layout(true)
            .imageless_framebuffer(true)
            .uniform_buffer_standard_layout(true)
            .shader_subgroup_extended_types(true)
            .separate_depth_stencil_layouts(true)
            .host_query_reset(true)
            .timeline_semaphore(capabilities.supports_timeline_semaphores)
            .buffer_device_address(true)
            .buffer_device_address_capture_replay(false)
            .buffer_device_address_multi_device(false)
            .vulkan_memory_model(true)
            .vulkan_memory_model_device_scope(true)
            .vulkan_memory_model_availability_visibility_chains(true)
            .shader_output_viewport_index(true)
            .shader_output_layer(true)
            .subgroup_broadcast_dynamic_id(true);

        let mut features13 = vk::PhysicalDeviceVulkan13Features::builder()
            .robust_image_access(true)
            .inline_uniform_block(true)
            .descriptor_binding_inline_uniform_block_update_after_bind(true)
            .pipeline_creation_cache_control(true)
            .private_data(true)
            .shader_demote_to_helper_invocation(true)
            .shader_terminate_invocation(true)
            .subgroup_size_control(true)
            .compute_full_subgroups(true)
            .synchronization2(true)
            .texture_compression_astc_hdr(true)
            .shader_zero_initialize_workgroup_memory(true)
            .dynamic_rendering(true)
            .shader_integer_dot_product(true)
            .maintenance4(true);

        let extensions = Self::get_required_extensions();
        let extension_names: Vec<*const i8> = extensions.iter().map(|ext| ext.as_ptr()).collect();

        let device_create_info = vk::DeviceCreateInfo::builder()
            .queue_create_infos(&queue_create_infos)
            .enabled_extension_names(&extension_names)
            .enabled_features(&device_features)
            .push_next(&mut features11)
            .push_next(&mut features12)
            .push_next(&mut features13);

        let device = unsafe {
            instance
                .create_device(physical_device, &device_create_info, None)
                .map_err(|e| DeviceError::DeviceCreation(e.to_string()))?
        };

        let graphics_queue = unsafe { device.get_device_queue(queue_families.graphics, 0) };
        let compute_queue = unsafe { device.get_device_queue(queue_families.compute, 0) };
        let transfer_queue = unsafe { device.get_device_queue(queue_families.transfer, 0) };
        let present_queue = queue_families
            .present
            .map(|index| unsafe { device.get_device_queue(index, 0) });

        Ok((
            device,
            (graphics_queue, compute_queue, transfer_queue, present_queue),
        ))
    }

    fn get_required_extensions() -> Vec<&'static CStr> {
        vec![
            ash::extensions::khr::Swapchain::name(),
            ash::extensions::khr::DynamicRendering::name(),
            ash::extensions::khr::BufferDeviceAddress::name(),
            ash::extensions::khr::TimelineSemaphore::name(),
            ash::extensions::khr::Synchronization2::name(),
            ash::extensions::khr::PushDescriptor::name(),
            // Use raw extension names for extensions not available as separate structs
            unsafe { c"VK_EXT_descriptor_indexing" },
            unsafe { c"VK_EXT_shader_viewport_index_layer" },
        ]
    }

    fn get_supported_extensions(
        instance: &Instance,
        device: vk::PhysicalDevice,
    ) -> Result<HashSet<String>> {
        let extensions = unsafe {
            instance
                .enumerate_device_extension_properties(device)
                .map_err(|e| DeviceError::ExtensionNotSupported(e.to_string()))?
        };

        Ok(extensions
            .iter()
            .map(|ext| {
                unsafe { CStr::from_ptr(ext.extension_name.as_ptr()) }
                    .to_string_lossy()
                    .to_string()
            })
            .collect())
    }

    pub fn physical_device(&self) -> vk::PhysicalDevice {
        self.physical_device
    }

    pub fn logical_device(&self) -> &Device {
        &self.logical_device
    }

    pub fn graphics_queue(&self) -> vk::Queue {
        self.graphics_queue
    }

    pub fn compute_queue(&self) -> vk::Queue {
        self.compute_queue
    }

    pub fn transfer_queue(&self) -> vk::Queue {
        self.transfer_queue
    }

    pub fn present_queue(&self) -> Option<vk::Queue> {
        self.present_queue
    }

    pub fn queue_families(&self) -> &QueueFamilyIndices {
        &self.queue_families
    }

    pub fn capabilities(&self) -> &DeviceCapabilities {
        &self.capabilities
    }

    pub fn memory_properties(&self) -> &vk::PhysicalDeviceMemoryProperties {
        &self.memory_properties
    }

    pub fn device_properties(&self) -> &vk::PhysicalDeviceProperties {
        &self.device_properties
    }

    pub fn find_memory_type(
        &self,
        type_filter: u32,
        properties: vk::MemoryPropertyFlags,
    ) -> Option<u32> {
        (0..self.memory_properties.memory_type_count).find(|&i| {
            (type_filter & (1 << i)) != 0
                && self.memory_properties.memory_types[i as usize]
                    .property_flags
                    .contains(properties)
        })
    }

    pub async fn wait_idle(&self) -> Result<()> {
        unsafe {
            self.logical_device
                .device_wait_idle()
                .map_err(|e| DeviceError::DeviceCreation(e.to_string()))?;
        }
        Ok(())
    }

    pub async fn wait_for_fences(
        &self,
        fences: &[vk::Fence],
        wait_all: bool,
        timeout: u64,
    ) -> Result<()> {
        unsafe {
            self.logical_device
                .wait_for_fences(fences, wait_all, timeout)
                .map_err(|e| DeviceError::DeviceCreation(e.to_string()))?;
        }
        Ok(())
    }

    pub fn reset_fences(&self, fences: &[vk::Fence]) -> Result<()> {
        unsafe {
            self.logical_device
                .reset_fences(fences)
                .map_err(|e| DeviceError::DeviceCreation(e.to_string()))?;
        }
        Ok(())
    }

    pub fn get_buffer_memory_requirements(&self, buffer: vk::Buffer) -> vk::MemoryRequirements {
        unsafe { self.logical_device.get_buffer_memory_requirements(buffer) }
    }

    pub fn get_image_memory_requirements(&self, image: vk::Image) -> vk::MemoryRequirements {
        unsafe { self.logical_device.get_image_memory_requirements(image) }
    }
}
