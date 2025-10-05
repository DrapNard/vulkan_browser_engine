pub mod channel;

pub use channel::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, Semaphore};
use tracing::log;
use uuid::Uuid;

pub struct IpcManager {
    channels: Arc<RwLock<HashMap<ChannelId, IpcChannel>>>,
    message_router: Arc<MessageRouter>,
    security_filter: Arc<SecurityFilter>,
    shutdown_signal: Arc<tokio::sync::Notify>,
}

pub type ChannelId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct IpcMessage {
    pub priority: MessagePriority,
    pub timestamp: u64,
    pub id: Uuid,
    pub sender: u32,
    pub recipient: u32,
    pub message_type: MessageType,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MessageType {
    ProcessControl,
    SecurityAudit,
    PermissionRequest,
    RenderCommand,
    DomUpdate,
    JavaScriptExecution,
    ResourceRequest,
    Custom(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessagePriority {
    Critical = 0,
    High = 1,
    Normal = 2,
    Low = 3,
}

struct MessageRouter {
    routes: Arc<RwLock<HashMap<u32, Arc<ProcessHandler>>>>,
    dead_letter_queue: mpsc::UnboundedSender<IpcMessage>,
    message_counter: Arc<std::sync::atomic::AtomicU64>,
}

struct ProcessHandler {
    sender: mpsc::UnboundedSender<IpcMessage>,
    priority_queues: [Arc<tokio::sync::Mutex<Vec<IpcMessage>>>; 4],
    processing_semaphore: Arc<Semaphore>,
}

struct SecurityFilter {
    message_limits: HashMap<MessageType, MessageLimits>,
    process_rate_limiters: Arc<RwLock<HashMap<u32, Arc<RateLimiter>>>>,
    global_rate_limiter: Arc<RateLimiter>,
}

#[derive(Clone)]
struct MessageLimits {
    max_size: usize,
    max_per_second: u32,
}

#[allow(dead_code)]
struct RateLimiter {
    max_tokens: u32,
    tokens: Arc<std::sync::atomic::AtomicU32>,
    last_refill: Arc<std::sync::atomic::AtomicU64>,
    refill_rate: u32,
}

impl IpcManager {
    pub fn new() -> Self {
        let (dead_letter_sender, mut dead_letter_receiver) =
            mpsc::unbounded_channel::<IpcMessage>();

        let shutdown_signal = Arc::new(tokio::sync::Notify::new());
        let shutdown_clone = Arc::clone(&shutdown_signal);

        tokio::spawn(async move {
            tokio::select! {
                _ = shutdown_clone.notified() => {
                    log::info!("Dead letter queue shutting down");
                }
                _ = async {
                    while let Some(message) = dead_letter_receiver.recv().await {
                        log::error!(
                            "Dead letter: Message {} from process {} to process {} could not be delivered",
                            message.id, message.sender, message.recipient
                        );
                    }
                } => {}
            }
        });

        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            message_router: Arc::new(MessageRouter::new(dead_letter_sender)),
            security_filter: Arc::new(SecurityFilter::new()),
            shutdown_signal,
        }
    }

    pub async fn create_channel(
        &self,
        process_a: u32,
        process_b: u32,
    ) -> Result<ChannelId, IpcError> {
        let channel_id = Uuid::new_v4();
        let channel = IpcChannel::new(process_a, process_b).await?;

        self.channels.write().await.insert(channel_id, channel);

        self.message_router.register_process(process_a).await?;
        self.message_router.register_process(process_b).await?;

        log::info!(
            "Created IPC channel {} between processes {} and {}",
            channel_id,
            process_a,
            process_b
        );
        Ok(channel_id)
    }

    pub async fn send_message(
        &self,
        from: u32,
        to: u32,
        mut message: IpcMessage,
    ) -> Result<(), IpcError> {
        message.sender = from;
        message.recipient = to;
        message.timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| IpcError::TimestampError)?
            .as_millis() as u64;

        self.security_filter.validate_message(&message).await?;
        self.message_router.route_message(message).await
    }

    pub async fn shutdown_channel(&self, channel_id: ChannelId) -> Result<(), IpcError> {
        if let Some(channel) = self.channels.write().await.remove(&channel_id) {
            channel.shutdown().await?;
            log::info!("Channel {} shut down successfully", channel_id);
        }
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), IpcError> {
        log::info!("Shutting down IPC Manager");

        let channels = std::mem::take(&mut *self.channels.write().await);
        for (id, channel) in channels {
            if let Err(e) = channel.shutdown().await {
                log::error!("Failed to shutdown channel {}: {}", id, e);
            }
        }

        self.shutdown_signal.notify_waiters();
        Ok(())
    }

    pub async fn get_stats(&self) -> IpcStats {
        let channels = self.channels.read().await;
        let total_channels = channels.len();
        let mut total_messages = 0;
        let mut total_bytes = 0;

        for channel in channels.values() {
            let stats = channel.get_stats().await;
            total_messages += stats.messages_sent + stats.messages_received;
            total_bytes += stats.bytes_sent + stats.bytes_received;
        }

        IpcStats {
            total_channels,
            total_messages,
            total_bytes,
            active_processes: self.message_router.routes.read().await.len(),
            messages_routed: self.message_router.get_message_count(),
        }
    }

    pub async fn get_process_stats(&self, process_id: u32) -> Option<ProcessStats> {
        let routes = self.message_router.routes.read().await;
        routes.get(&process_id).map(|handler| ProcessStats {
            process_id,
            pending_messages: handler.get_pending_count(),
            processing_capacity: handler.processing_semaphore.available_permits(),
        })
    }
}

impl MessageRouter {
    fn new(dead_letter_queue: mpsc::UnboundedSender<IpcMessage>) -> Self {
        Self {
            routes: Arc::new(RwLock::new(HashMap::new())),
            dead_letter_queue,
            message_counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    async fn register_process(&self, process_id: u32) -> Result<(), IpcError> {
        let routes_read = self.routes.read().await;
        if routes_read.contains_key(&process_id) {
            return Ok(());
        }
        drop(routes_read);

        let (sender, receiver) = mpsc::unbounded_channel::<IpcMessage>();
        let handler = Arc::new(ProcessHandler::new(sender));

        self.routes
            .write()
            .await
            .insert(process_id, Arc::clone(&handler));
        self.spawn_process_worker(process_id, receiver, Arc::clone(&handler))
            .await;
        Ok(())
    }

    async fn spawn_process_worker(
        &self,
        process_id: u32,
        mut receiver: mpsc::UnboundedReceiver<IpcMessage>,
        handler: Arc<ProcessHandler>,
    ) {
        tokio::spawn(async move {
            while let Some(message) = receiver.recv().await {
                let priority_index = message.priority as usize;

                if let Some(queue) = handler.priority_queues.get(priority_index) {
                    queue.lock().await.push(message);

                    if let Ok(_permit) = handler.processing_semaphore.try_acquire() {
                        Self::process_priority_queues(process_id, &handler).await;
                    }
                }
            }

            log::info!("Process worker {} shutting down", process_id);
        });
    }

    async fn process_priority_queues(process_id: u32, handler: &ProcessHandler) {
        for priority_queue in &handler.priority_queues {
            let mut queue = priority_queue.lock().await;

            while let Some(message) = queue.pop() {
                if let Err(e) = Self::deliver_message(process_id, message).await {
                    log::error!("Failed to deliver message to process {}: {}", process_id, e);
                }
            }
        }
    }

    async fn deliver_message(_process_id: u32, _message: IpcMessage) -> Result<(), IpcError> {
        Ok(())
    }

    async fn route_message(&self, message: IpcMessage) -> Result<(), IpcError> {
        self.message_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let routes = self.routes.read().await;
        if let Some(handler) = routes.get(&message.recipient) {
            handler
                .sender
                .send(message)
                .map_err(|_| IpcError::MessageDeliveryFailed)?;
        } else {
            drop(routes);
            self.dead_letter_queue
                .send(message)
                .map_err(|_| IpcError::MessageDeliveryFailed)?;
        }
        Ok(())
    }

    fn get_message_count(&self) -> u64 {
        self.message_counter
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl ProcessHandler {
    fn new(sender: mpsc::UnboundedSender<IpcMessage>) -> Self {
        Self {
            sender,
            priority_queues: [
                Arc::new(tokio::sync::Mutex::new(Vec::new())),
                Arc::new(tokio::sync::Mutex::new(Vec::new())),
                Arc::new(tokio::sync::Mutex::new(Vec::new())),
                Arc::new(tokio::sync::Mutex::new(Vec::new())),
            ],
            processing_semaphore: Arc::new(Semaphore::new(10)),
        }
    }

    fn get_pending_count(&self) -> usize {
        self.priority_queues.len()
    }
}

impl SecurityFilter {
    fn new() -> Self {
        let mut message_limits = HashMap::new();

        message_limits.insert(
            MessageType::RenderCommand,
            MessageLimits {
                max_size: 1024 * 1024,
                max_per_second: 100,
            },
        );
        message_limits.insert(
            MessageType::DomUpdate,
            MessageLimits {
                max_size: 512 * 1024,
                max_per_second: 200,
            },
        );
        message_limits.insert(
            MessageType::JavaScriptExecution,
            MessageLimits {
                max_size: 2 * 1024 * 1024,
                max_per_second: 50,
            },
        );
        message_limits.insert(
            MessageType::ResourceRequest,
            MessageLimits {
                max_size: 64 * 1024,
                max_per_second: 500,
            },
        );

        Self {
            message_limits,
            process_rate_limiters: Arc::new(RwLock::new(HashMap::new())),
            global_rate_limiter: Arc::new(RateLimiter::new(10000)),
        }
    }

    async fn validate_message(&self, message: &IpcMessage) -> Result<(), IpcError> {
        if !self.global_rate_limiter.check_rate().await {
            return Err(IpcError::SecurityViolation(
                "Global rate limit exceeded".to_string(),
            ));
        }

        if let Some(limits) = self.message_limits.get(&message.message_type) {
            if message.payload.len() > limits.max_size {
                return Err(IpcError::SecurityViolation(format!(
                    "Message size {} exceeds limit {}",
                    message.payload.len(),
                    limits.max_size
                )));
            }

            let rate_limiter = self
                .get_or_create_rate_limiter(message.sender, limits.max_per_second)
                .await;
            if !rate_limiter.check_rate().await {
                return Err(IpcError::SecurityViolation(format!(
                    "Rate limit exceeded for process {}",
                    message.sender
                )));
            }
        }

        Ok(())
    }

    async fn get_or_create_rate_limiter(
        &self,
        process_id: u32,
        max_per_second: u32,
    ) -> Arc<RateLimiter> {
        let limiters = self.process_rate_limiters.read().await;

        if let Some(limiter) = limiters.get(&process_id) {
            Arc::clone(limiter)
        } else {
            drop(limiters);

            let mut limiters = self.process_rate_limiters.write().await;
            let limiter = Arc::new(RateLimiter::new(max_per_second));
            limiters.insert(process_id, Arc::clone(&limiter));
            limiter
        }
    }
}

impl RateLimiter {
    fn new(max_per_second: u32) -> Self {
        Self {
            max_tokens: max_per_second,
            tokens: Arc::new(std::sync::atomic::AtomicU32::new(max_per_second)),
            last_refill: Arc::new(std::sync::atomic::AtomicU64::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            )),
            refill_rate: max_per_second,
        }
    }

    async fn check_rate(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let last_refill = self.last_refill.load(std::sync::atomic::Ordering::Relaxed);

        if now > last_refill + 1000 {
            self.last_refill
                .store(now, std::sync::atomic::Ordering::Relaxed);
            self.tokens
                .store(self.max_tokens, std::sync::atomic::Ordering::Relaxed);
        }

        let current_tokens = self.tokens.load(std::sync::atomic::Ordering::Relaxed);
        if current_tokens > 0 {
            self.tokens
                .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone)]
pub struct IpcStats {
    pub total_channels: usize,
    pub total_messages: u64,
    pub total_bytes: u64,
    pub active_processes: usize,
    pub messages_routed: u64,
}

#[derive(Debug, Clone)]
pub struct ProcessStats {
    pub process_id: u32,
    pub pending_messages: usize,
    pub processing_capacity: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("Channel creation failed: {0}")]
    ChannelCreationFailed(String),
    #[error("Message delivery failed")]
    MessageDeliveryFailed,
    #[error("Security violation: {0}")]
    SecurityViolation(String),
    #[error("Timestamp error")]
    TimestampError,
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("Channel not found")]
    ChannelNotFound,
    #[error("Process not found: {0}")]
    ProcessNotFound(u32),
    #[error("Rate limit exceeded")]
    RateLimitExceeded,
}

impl Default for IpcManager {
    fn default() -> Self {
        Self::new()
    }
}
