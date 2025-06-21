pub mod channel;

pub use channel::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

pub struct IpcManager {
    channels: Arc<RwLock<HashMap<ChannelId, IpcChannel>>>,
    message_router: MessageRouter,
    security_filter: SecurityFilter,
}

pub type ChannelId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcMessage {
    pub id: Uuid,
    pub sender: u32,
    pub recipient: u32,
    pub message_type: MessageType,
    pub payload: Vec<u8>,
    pub timestamp: u64,
    pub priority: MessagePriority,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum MessageType {
    RenderCommand,
    DomUpdate,
    JavaScriptExecution,
    ResourceRequest,
    SecurityAudit,
    PermissionRequest,
    ProcessControl,
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
    routes: Arc<RwLock<HashMap<u32, mpsc::UnboundedSender<IpcMessage>>>>,
    dead_letter_queue: mpsc::UnboundedSender<IpcMessage>,
}

struct SecurityFilter {
    allowed_message_types: HashMap<(u32, u32), Vec<MessageType>>,
    message_size_limits: HashMap<MessageType, usize>,
    rate_limits: HashMap<u32, RateLimiter>,
}

struct RateLimiter {
    max_messages_per_second: u32,
    current_count: Arc<RwLock<u32>>,
    last_reset: Arc<RwLock<std::time::Instant>>,
}

impl IpcManager {
    pub fn new() -> Self {
        let (dead_letter_sender, mut dead_letter_receiver) = mpsc::unbounded_channel();
        
        tokio::spawn(async move {
            while let Some(message) = dead_letter_receiver.recv().await {
                log::error!("Dead letter: Message {} from {} could not be delivered", 
                    message.id, message.sender);
            }
        });

        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            message_router: MessageRouter {
                routes: Arc::new(RwLock::new(HashMap::new())),
                dead_letter_queue: dead_letter_sender,
            },
            security_filter: SecurityFilter::new(),
        }
    }

    pub async fn create_channel(&self, process_a: u32, process_b: u32) -> Result<ChannelId, IpcError> {
        let channel_id = Uuid::new_v4();
        let channel = IpcChannel::new(process_a, process_b).await?;
        
        {
            let mut channels = self.channels.write().await;
            channels.insert(channel_id, channel);
        }

        self.register_process_route(process_a).await?;
        self.register_process_route(process_b).await?;

        Ok(channel_id)
    }

    pub async fn send_message(&self, from: u32, to: u32, mut message: IpcMessage) -> Result<(), IpcError> {
        message.sender = from;
        message.recipient = to;
        message.timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| IpcError::TimestampError)?
            .as_millis() as u64;

        if !self.security_filter.validate_message(&message).await? {
            return Err(IpcError::SecurityViolation(format!(
                "Message blocked by security filter: {} -> {}", from, to
            )));
        }

        self.message_router.route_message(message).await
    }

    async fn register_process_route(&self, process_id: u32) -> Result<(), IpcError> {
        let routes = self.message_router.routes.read().await;
        if !routes.contains_key(&process_id) {
            drop(routes);
            
            let (sender, receiver) = mpsc::unbounded_channel();
            {
                let mut routes = self.message_router.routes.write().await;
                routes.insert(process_id, sender);
            }
            
            self.spawn_message_handler(process_id, receiver).await;
        }
        Ok(())
    }

    async fn spawn_message_handler(&self, process_id: u32, mut receiver: mpsc::UnboundedReceiver<IpcMessage>) {
        tokio::spawn(async move {
            let mut priority_queue = std::collections::BinaryHeap::new();
            
            while let Some(message) = receiver.recv().await {
                priority_queue.push(std::cmp::Reverse((message.priority, message)));
                
                while let Some(std::cmp::Reverse((_, msg))) = priority_queue.pop() {
                    if let Err(e) = Self::deliver_message(process_id, msg).await {
                        log::error!("Failed to deliver message to process {}: {}", process_id, e);
                    }
                }
            }
        });
    }

    async fn deliver_message(process_id: u32, message: IpcMessage) -> Result<(), IpcError> {
        Ok(())
    }

    pub async fn shutdown_channel(&self, channel_id: ChannelId) -> Result<(), IpcError> {
        let mut channels = self.channels.write().await;
        if let Some(channel) = channels.remove(&channel_id) {
            channel.shutdown().await?;
        }
        Ok(())
    }

    pub async fn get_channel_stats(&self) -> IpcStats {
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
        }
    }
}

impl SecurityFilter {
    fn new() -> Self {
        let mut allowed_types = HashMap::new();
        let mut size_limits = HashMap::new();
        
        size_limits.insert(MessageType::RenderCommand, 1024 * 1024);
        size_limits.insert(MessageType::DomUpdate, 512 * 1024);
        size_limits.insert(MessageType::JavaScriptExecution, 2 * 1024 * 1024);
        size_limits.insert(MessageType::ResourceRequest, 64 * 1024);

        Self {
            allowed_message_types: allowed_types,
            message_size_limits: size_limits,
            rate_limits: HashMap::new(),
        }
    }

    async fn validate_message(&self, message: &IpcMessage) -> Result<bool, IpcError> {
        if let Some(limit) = self.message_size_limits.get(&message.message_type) {
            if message.payload.len() > *limit {
                return Ok(false);
            }
        }

        if let Some(rate_limiter) = self.rate_limits.get(&message.sender) {
            if !rate_limiter.check_rate().await {
                return Ok(false);
            }
        }

        Ok(true)
    }
}

impl RateLimiter {
    fn new(max_per_second: u32) -> Self {
        Self {
            max_messages_per_second: max_per_second,
            current_count: Arc::new(RwLock::new(0)),
            last_reset: Arc::new(RwLock::new(std::time::Instant::now())),
        }
    }

    async fn check_rate(&self) -> bool {
        let now = std::time::Instant::now();
        let mut last_reset = self.last_reset.write().await;
        
        if now.duration_since(*last_reset).as_secs() >= 1 {
            *last_reset = now;
            *self.current_count.write().await = 0;
        }
        
        let mut count = self.current_count.write().await;
        if *count < self.max_messages_per_second {
            *count += 1;
            true
        } else {
            false
        }
    }
}

impl MessageRouter {
    async fn route_message(&self, message: IpcMessage) -> Result<(), IpcError> {
        let routes = self.routes.read().await;
        if let Some(sender) = routes.get(&message.recipient) {
            sender.send(message)
                .map_err(|_| IpcError::MessageDeliveryFailed)?;
        } else {
            self.dead_letter_queue.send(message)
                .map_err(|_| IpcError::MessageDeliveryFailed)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct IpcStats {
    pub total_channels: usize,
    pub total_messages: u64,
    pub total_bytes: u64,
    pub active_processes: usize,
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
}

impl Default for IpcManager {
    fn default() -> Self {
        Self::new()
    }
}