use std::sync::Arc;
use std::collections::VecDeque;
use ash::{Device, vk};
use parking_lot::{Mutex, RwLock};
use crossbeam::channel::{Sender, unbounded};
use dashmap::DashMap;
use smallvec::SmallVec;
use thiserror::Error;
use super::device::VulkanDevice;

#[derive(Error, Debug)]
pub enum CommandError {
    #[error("Pool creation failed: {0}")]
    PoolCreation(String),
    #[error("Buffer allocation failed: {0}")]
    BufferAllocation(String),
    #[error("Command recording failed: {0}")]
    Recording(String),
    #[error("Submission failed: {0}")]
    Submission(String),
    #[error("Synchronization failed: {0}")]
    Synchronization(String),
}

pub type Result<T> = std::result::Result<T, CommandError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandBufferType {
    Graphics,
    Compute,
    Transfer,
}

#[derive(Debug, Clone, Copy)]
pub struct CommandBufferInfo {
    pub buffer: vk::CommandBuffer,
    pub pool: vk::CommandPool,
    pub queue_family: u32,
    pub buffer_type: CommandBufferType,
    pub is_recording: bool,
    pub frame_index: u32,
}

struct CommandPool {
    pool: vk::CommandPool,
    queue_family: u32,
    available_buffers: VecDeque<vk::CommandBuffer>,
    in_use_buffers: Vec<vk::CommandBuffer>,
    reset_flags: vk::CommandPoolResetFlags,
}

impl CommandPool {
    fn new(
        device: &Device,
        queue_family: u32,
        flags: vk::CommandPoolCreateFlags
    ) -> Result<Self> {
        let pool_info = vk::CommandPoolCreateInfo::builder()
            .flags(flags)
            .queue_family_index(queue_family);

        let pool = unsafe {
            device.create_command_pool(&pool_info, None)
                .map_err(|e| CommandError::PoolCreation(e.to_string()))?
        };

        Ok(Self {
            pool,
            queue_family,
            available_buffers: VecDeque::with_capacity(64),
            in_use_buffers: Vec::with_capacity(64),
            reset_flags: vk::CommandPoolResetFlags::empty(),
        })
    }

    fn allocate_buffer(&mut self, device: &Device, level: vk::CommandBufferLevel) -> Result<vk::CommandBuffer> {
        if let Some(buffer) = self.available_buffers.pop_front() {
            self.in_use_buffers.push(buffer);
            return Ok(buffer);
        }

        let alloc_info = vk::CommandBufferAllocateInfo::builder()
            .command_pool(self.pool)
            .level(level)
            .command_buffer_count(1);

        let buffers = unsafe {
            device.allocate_command_buffers(&alloc_info)
                .map_err(|e| CommandError::BufferAllocation(e.to_string()))?
        };

        let buffer = buffers[0];
        self.in_use_buffers.push(buffer);
        Ok(buffer)
    }

    fn reset_pool(&mut self, device: &Device) -> Result<()> {
        unsafe {
            device.reset_command_pool(self.pool, self.reset_flags)
                .map_err(|e| CommandError::Recording(e.to_string()))?;
        }

        self.available_buffers.extend(self.in_use_buffers.drain(..));
        Ok(())
    }

    fn return_buffer(&mut self, buffer: vk::CommandBuffer) {
        if let Some(pos) = self.in_use_buffers.iter().position(|&b| b == buffer) {
            self.in_use_buffers.remove(pos);
            self.available_buffers.push_back(buffer);
        }
    }
}

pub struct FrameData {
    pub command_buffers: SmallVec<[CommandBufferInfo; 16]>,
    pub submission_fence: vk::Fence,
    pub render_finished_semaphore: vk::Semaphore,
    pub image_available_semaphore: vk::Semaphore,
    pub frame_index: u32,
    pub is_submitted: bool,
}

impl FrameData {
    fn new(device: &Device, frame_index: u32) -> Result<Self> {
        let fence_info = vk::FenceCreateInfo::builder()
            .flags(vk::FenceCreateFlags::SIGNALED);

        let submission_fence = unsafe {
            device.create_fence(&fence_info, None)
                .map_err(|e| CommandError::Synchronization(e.to_string()))?
        };

        let semaphore_info = vk::SemaphoreCreateInfo::builder();

        let render_finished_semaphore = unsafe {
            device.create_semaphore(&semaphore_info, None)
                .map_err(|e| CommandError::Synchronization(e.to_string()))?
        };

        let image_available_semaphore = unsafe {
            device.create_semaphore(&semaphore_info, None)
                .map_err(|e| CommandError::Synchronization(e.to_string()))?
        };

        Ok(Self {
            command_buffers: SmallVec::new(),
            submission_fence,
            render_finished_semaphore,
            image_available_semaphore,
            frame_index,
            is_submitted: false,
        })
    }
}

pub struct CommandManager {
    device: Arc<VulkanDevice>,
    graphics_pools: Arc<Mutex<Vec<CommandPool>>>,
    compute_pools: Arc<Mutex<Vec<CommandPool>>>,
    transfer_pools: Arc<Mutex<Vec<CommandPool>>>,
    thread_local_pools: Arc<DashMap<std::thread::ThreadId, usize>>,
    frames_in_flight: Arc<RwLock<VecDeque<FrameData>>>,
    current_frame: Arc<RwLock<u32>>,
    max_frames_in_flight: u32,
    command_buffer_registry: Arc<DashMap<vk::CommandBuffer, CommandBufferInfo>>,
    submission_queue: Arc<Mutex<VecDeque<SubmissionBatch>>>,
    worker_thread: Option<std::thread::JoinHandle<()>>,
    shutdown_signal: Arc<Mutex<Option<Sender<()>>>>,
}

#[derive(Debug)]
struct SubmissionBatch {
    command_buffers: Vec<vk::CommandBuffer>,
    queue: vk::Queue,
    wait_semaphores: Vec<vk::Semaphore>,
    wait_stages: Vec<vk::PipelineStageFlags>,
    signal_semaphores: Vec<vk::Semaphore>,
    fence: vk::Fence,
}

impl CommandManager {
    pub async fn new(device: Arc<VulkanDevice>) -> Result<Self> {
        let max_frames_in_flight = 3;
        
        let graphics_pools = Arc::new(Mutex::new(Self::create_pools(
            device.logical_device(),
            device.queue_families().graphics,
            vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER,
            8
        )?));

        let compute_pools = Arc::new(Mutex::new(Self::create_pools(
            device.logical_device(),
            device.queue_families().compute,
            vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER,
            4
        )?));

        let transfer_pools = Arc::new(Mutex::new(Self::create_pools(
            device.logical_device(),
            device.queue_families().transfer,
            vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER | vk::CommandPoolCreateFlags::TRANSIENT,
            4
        )?));

        let mut frames_in_flight = VecDeque::with_capacity(max_frames_in_flight as usize);
        for i in 0..max_frames_in_flight {
            frames_in_flight.push_back(FrameData::new(device.logical_device(), i)?);
        }

        let (shutdown_tx, _shutdown_rx) = unbounded();

        let manager = Self {
            device: device.clone(),
            graphics_pools,
            compute_pools,
            transfer_pools,
            thread_local_pools: Arc::new(DashMap::new()),
            frames_in_flight: Arc::new(RwLock::new(frames_in_flight)),
            current_frame: Arc::new(RwLock::new(0)),
            max_frames_in_flight,
            command_buffer_registry: Arc::new(DashMap::new()),
            submission_queue: Arc::new(Mutex::new(VecDeque::new())),
            worker_thread: None,
            shutdown_signal: Arc::new(Mutex::new(Some(shutdown_tx))),
        };

        Ok(manager)
    }

    fn create_pools(
        device: &Device,
        queue_family: u32,
        flags: vk::CommandPoolCreateFlags,
        count: usize
    ) -> Result<Vec<CommandPool>> {
        let mut pools = Vec::with_capacity(count);
        for _ in 0..count {
            pools.push(CommandPool::new(device, queue_family, flags)?);
        }
        Ok(pools)
    }

    pub async fn begin_frame(&self) -> Result<vk::CommandBuffer> {
        let current_frame = {
            let mut frame = self.current_frame.write();
            let f = *frame;
            *frame = (*frame + 1) % self.max_frames_in_flight;
            f
        };

        let mut frames = self.frames_in_flight.write();
        let frame_data = &mut frames[current_frame as usize];

        unsafe {
            self.device.logical_device().wait_for_fences(
                &[frame_data.submission_fence],
                true,
                u64::MAX
            ).map_err(|e| CommandError::Synchronization(e.to_string()))?;

            self.device.logical_device().reset_fences(&[frame_data.submission_fence])
                .map_err(|e| CommandError::Synchronization(e.to_string()))?;
        }

        for buffer_info in &frame_data.command_buffers {
            self.command_buffer_registry.remove(&buffer_info.buffer);
        }
        frame_data.command_buffers.clear();
        frame_data.is_submitted = false;

        drop(frames);

        self.allocate_command_buffer(CommandBufferType::Graphics, vk::CommandBufferLevel::PRIMARY).await
    }

    pub async fn allocate_command_buffer(
        &self,
        buffer_type: CommandBufferType,
        level: vk::CommandBufferLevel
    ) -> Result<vk::CommandBuffer> {
        let thread_id = std::thread::current().id();
        
        let pool_index = self.thread_local_pools.get(&thread_id)
            .map(|entry| *entry.value())
            .unwrap_or_else(|| {
                let index = fastrand::usize(..8);
                self.thread_local_pools.insert(thread_id, index);
                index
            });

        let buffer = match buffer_type {
            CommandBufferType::Graphics => {
                let mut pools = self.graphics_pools.lock();
                let pool_count = pools.len();
                pools[pool_index % pool_count].allocate_buffer(self.device.logical_device(), level)?
            },
            CommandBufferType::Compute => {
                let mut pools = self.compute_pools.lock();
                let pool_count = pools.len();
                pools[pool_index % pool_count].allocate_buffer(self.device.logical_device(), level)?
            },
            CommandBufferType::Transfer => {
                let mut pools = self.transfer_pools.lock();
                let pool_count = pools.len();
                pools[pool_index % pool_count].allocate_buffer(self.device.logical_device(), level)?
            },
        };

        let queue_family = match buffer_type {
            CommandBufferType::Graphics => self.device.queue_families().graphics,
            CommandBufferType::Compute => self.device.queue_families().compute,
            CommandBufferType::Transfer => self.device.queue_families().transfer,
        };

        let current_frame = *self.current_frame.read();
        
        let buffer_info = CommandBufferInfo {
            buffer,
            pool: vk::CommandPool::null(),
            queue_family,
            buffer_type,
            is_recording: false,
            frame_index: current_frame,
        };

        self.command_buffer_registry.insert(buffer, buffer_info);

        let begin_info = vk::CommandBufferBeginInfo::builder()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

        unsafe {
            self.device.logical_device().begin_command_buffer(buffer, &begin_info)
                .map_err(|e| CommandError::Recording(e.to_string()))?;
        }

        if let Some(mut entry) = self.command_buffer_registry.get_mut(&buffer) {
            entry.is_recording = true;
        }

        Ok(buffer)
    }

    pub async fn end_command_buffer(&self, buffer: vk::CommandBuffer) -> Result<()> {
        unsafe {
            self.device.logical_device().end_command_buffer(buffer)
                .map_err(|e| CommandError::Recording(e.to_string()))?;
        }

        if let Some(mut entry) = self.command_buffer_registry.get_mut(&buffer) {
            entry.is_recording = false;
        }

        Ok(())
    }

    pub async fn submit_command_buffer(
        &self,
        buffer: vk::CommandBuffer,
        wait_semaphores: &[vk::Semaphore],
        wait_stages: &[vk::PipelineStageFlags],
        signal_semaphores: &[vk::Semaphore]
    ) -> Result<()> {
        let buffer_info = self.command_buffer_registry.get(&buffer)
            .ok_or_else(|| CommandError::Submission("Buffer not found in registry".to_string()))?;

        let queue = match buffer_info.buffer_type {
            CommandBufferType::Graphics => self.device.graphics_queue(),
            CommandBufferType::Compute => self.device.compute_queue(),
            CommandBufferType::Transfer => self.device.transfer_queue(),
        };

        // Get the submission fence, ensuring proper lifetime management
        let submission_fence = {
            let current_frame = *self.current_frame.read();
            let frame_index = ((current_frame + self.max_frames_in_flight - 1) % self.max_frames_in_flight) as usize;
            let frames = self.frames_in_flight.read();
            frames[frame_index].submission_fence
        };

        let command_buffers = [buffer];
        let submit_info = vk::SubmitInfo::builder()
            .command_buffers(&command_buffers)
            .wait_semaphores(wait_semaphores)
            .wait_dst_stage_mask(wait_stages)
            .signal_semaphores(signal_semaphores)
            .build();

        unsafe {
            self.device.logical_device().queue_submit(
                queue,
                &[submit_info],
                submission_fence
            ).map_err(|e| CommandError::Submission(e.to_string()))?;
        }

        Ok(())
    }

    pub async fn end_frame(&self, primary_buffer: vk::CommandBuffer) -> Result<()> {
        self.end_command_buffer(primary_buffer).await?;

        // Get the frame index and extract semaphore/fence values, ensuring proper lifetime management
        let (render_finished_semaphore, submission_fence) = {
            let current_frame = *self.current_frame.read();
            let frame_index = ((current_frame + self.max_frames_in_flight - 1) % self.max_frames_in_flight) as usize;
            let mut frames = self.frames_in_flight.write();
            let frame_data = &mut frames[frame_index];

            if let Some(buffer_info) = self.command_buffer_registry.get(&primary_buffer) {
                frame_data.command_buffers.push(*buffer_info.value());
            }

            let semaphore = frame_data.render_finished_semaphore;
            let fence = frame_data.submission_fence;
            
            // Extract values before releasing the lock
            (semaphore, fence)
        };

        let command_buffers = [primary_buffer];
        let signal_semaphores = [render_finished_semaphore];
        let submit_info = vk::SubmitInfo::builder()
            .command_buffers(&command_buffers)
            .signal_semaphores(&signal_semaphores)
            .build();

        unsafe {
            self.device.logical_device().queue_submit(
                self.device.graphics_queue(),
                &[submit_info],
                submission_fence
            ).map_err(|e| CommandError::Submission(e.to_string()))?;
        }

        // Update the submission status
        {
            let current_frame = *self.current_frame.read();
            let frame_index = ((current_frame + self.max_frames_in_flight - 1) % self.max_frames_in_flight) as usize;
            let mut frames = self.frames_in_flight.write();
            frames[frame_index].is_submitted = true;
        }

        Ok(())
    }

    pub async fn wait_for_frame(&self, frame_index: u32) -> Result<()> {
        let frames = self.frames_in_flight.read();
        let frame_data = &frames[(frame_index % self.max_frames_in_flight) as usize];

        if frame_data.is_submitted {
            unsafe {
                self.device.logical_device().wait_for_fences(
                    &[frame_data.submission_fence],
                    true,
                    u64::MAX
                ).map_err(|e| CommandError::Synchronization(e.to_string()))?;
            }
        }

        Ok(())
    }

    pub async fn reset_pools(&self) -> Result<()> {
        let mut graphics_pools = self.graphics_pools.lock();
        for pool in graphics_pools.iter_mut() {
            pool.reset_pool(self.device.logical_device())?;
        }

        let mut compute_pools = self.compute_pools.lock();
        for pool in compute_pools.iter_mut() {
            pool.reset_pool(self.device.logical_device())?;
        }

        let mut transfer_pools = self.transfer_pools.lock();
        for pool in transfer_pools.iter_mut() {
            pool.reset_pool(self.device.logical_device())?;
        }

        Ok(())
    }

    pub fn get_frame_semaphores(&self, frame_index: u32) -> (vk::Semaphore, vk::Semaphore) {
        let frames = self.frames_in_flight.read();
        let frame_data = &frames[(frame_index % self.max_frames_in_flight) as usize];
        (frame_data.image_available_semaphore, frame_data.render_finished_semaphore)
    }

    pub async fn shutdown(&self) -> Result<()> {
        if let Some(sender) = self.shutdown_signal.lock().take() {
            let _ = sender.send(());
        }

        self.device.wait_idle().await.map_err(|e| CommandError::Synchronization(e.to_string()))?;

        let frames = self.frames_in_flight.read();
        for frame_data in frames.iter() {
            unsafe {
                self.device.logical_device().destroy_fence(frame_data.submission_fence, None);
                self.device.logical_device().destroy_semaphore(frame_data.render_finished_semaphore, None);
                self.device.logical_device().destroy_semaphore(frame_data.image_available_semaphore, None);
            }
        }

        let graphics_pools = self.graphics_pools.lock();
        for pool in graphics_pools.iter() {
            unsafe {
                self.device.logical_device().destroy_command_pool(pool.pool, None);
            }
        }

        let compute_pools = self.compute_pools.lock();
        for pool in compute_pools.iter() {
            unsafe {
                self.device.logical_device().destroy_command_pool(pool.pool, None);
            }
        }

        let transfer_pools = self.transfer_pools.lock();
        for pool in transfer_pools.iter() {
            unsafe {
                self.device.logical_device().destroy_command_pool(pool.pool, None);
            }
        }

        Ok(())
    }
}