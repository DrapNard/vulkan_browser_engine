src/sandbox/ipc/mod.rs
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

src/sandbox/ipc/channel.rs
use super::{IpcError, IpcMessage, MessagePriority};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};

pub struct IpcChannel {
    process_a: u32,
    process_b: u32,
    sender_a: mpsc::UnboundedSender<IpcMessage>,
    sender_b: mpsc::UnboundedSender<IpcMessage>,
    stats: Arc<RwLock<ChannelStats>>,
    message_filter: MessageFilter,
    encryption_key: Option<[u8; 32]>,
}

#[derive(Debug, Default)]
pub struct ChannelStats {
    pub messages_sent: u64,
    pub messages_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub errors: u64,
    pub last_activity: Option<std::time::Instant>,
}

struct MessageFilter {
    max_message_size: usize,
    allowed_priorities: Vec<MessagePriority>,
    blocked_senders: std::collections::HashSet<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EncryptedMessage {
    nonce: [u8; 12],
    ciphertext: Vec<u8>,
    tag: [u8; 16],
}

impl IpcChannel {
    pub async fn new(process_a: u32, process_b: u32) -> Result<Self, IpcError> {
        let (sender_a, mut receiver_a) = mpsc::unbounded_channel();
        let (sender_b, mut receiver_b) = mpsc::unbounded_channel();
        
        let stats = Arc::new(RwLock::new(ChannelStats::default()));
        let stats_clone_a = stats.clone();
        let stats_clone_b = stats.clone();

        tokio::spawn(async move {
            while let Some(message) = receiver_a.recv().await {
                Self::handle_message(process_a, message, stats_clone_a.clone()).await;
            }
        });

        tokio::spawn(async move {
            while let Some(message) = receiver_b.recv().await {
                Self::handle_message(process_b, message, stats_clone_b.clone()).await;
            }
        });

        Ok(Self {
            process_a,
            process_b,
            sender_a,
            sender_b,
            stats,
            message_filter: MessageFilter::new(),
            encryption_key: Self::generate_encryption_key(),
        })
    }

    pub async fn send_message(&self, from: u32, message: IpcMessage) -> Result<(), IpcError> {
        if !self.message_filter.validate_message(&message) {
            let mut stats = self.stats.write().await;
            stats.errors += 1;
            return Err(IpcError::SecurityViolation("Message filtered".to_string()));
        }

        let encrypted_message = if let Some(key) = &self.encryption_key {
            self.encrypt_message(&message, key)?
        } else {
            message
        };

        let sender = if from == self.process_a {
            &self.sender_b
        } else if from == self.process_b {
            &self.sender_a
        } else {
            return Err(IpcError::SecurityViolation("Unauthorized sender".to_string()));
        };

        sender.send(encrypted_message)
            .map_err(|_| IpcError::MessageDeliveryFailed)?;

        let mut stats = self.stats.write().await;
        stats.messages_sent += 1;
        stats.bytes_sent += message.payload.len() as u64;
        stats.last_activity = Some(std::time::Instant::now());

        Ok(())
    }

    async fn handle_message(process_id: u32, message: IpcMessage, stats: Arc<RwLock<ChannelStats>>) {
        let mut stats_guard = stats.write().await;
        stats_guard.messages_received += 1;
        stats_guard.bytes_received += message.payload.len() as u64;
        stats_guard.last_activity = Some(std::time::Instant::now());
        
        log::debug!("Process {} received message {}", process_id, message.id);
    }

    fn encrypt_message(&self, message: &IpcMessage, key: &[u8; 32]) -> Result<IpcMessage, IpcError> {
        use aes_gcm::{Aes256Gcm, Key, Nonce, NewAead, Aead};
        
        let cipher = Aes256Gcm::new(Key::from_slice(key));
        let nonce_bytes = rand::random::<[u8; 12]>();
        let nonce = Nonce::from_slice(&nonce_bytes);
        
        let serialized = bincode::serialize(message)
            .map_err(|e| IpcError::SerializationError(e.to_string()))?;
        
        let ciphertext = cipher.encrypt(nonce, serialized.as_slice())
            .map_err(|_| IpcError::SecurityViolation("Encryption failed".to_string()))?;

        let tag = &ciphertext[ciphertext.len() - 16..];
        let encrypted_payload = &ciphertext[..ciphertext.len() - 16];

        let encrypted_msg = EncryptedMessage {
            nonce: nonce_bytes,
            ciphertext: encrypted_payload.to_vec(),
            tag: tag.try_into().map_err(|_| IpcError::SecurityViolation("Invalid tag".to_string()))?,
        };

        let encrypted_payload = bincode::serialize(&encrypted_msg)
            .map_err(|e| IpcError::SerializationError(e.to_string()))?;

        Ok(IpcMessage {
            id: message.id,
            sender: message.sender,
            recipient: message.recipient,
            message_type: message.message_type.clone(),
            payload: encrypted_payload,
            timestamp: message.timestamp,
            priority: message.priority,
        })
    }

    fn generate_encryption_key() -> Option<[u8; 32]> {
        Some(rand::random())
    }

    pub async fn get_stats(&self) -> ChannelStats {
        self.stats.read().await.clone()
    }

    pub async fn shutdown(&self) -> Result<(), IpcError> {
        Ok(())
    }

    pub fn get_participants(&self) -> (u32, u32) {
        (self.process_a, self.process_b)
    }

    pub async fn set_encryption(&mut self, enabled: bool) {
        self.encryption_key = if enabled {
            Self::generate_encryption_key()
        } else {
            None
        };
    }
}

impl MessageFilter {
    fn new() -> Self {
        Self {
            max_message_size: 16 * 1024 * 1024,
            allowed_priorities: vec![
                MessagePriority::Critical,
                MessagePriority::High,
                MessagePriority::Normal,
                MessagePriority::Low,
            ],
            blocked_senders: std::collections::HashSet::new(),
        }
    }

    fn validate_message(&self, message: &IpcMessage) -> bool {
        if message.payload.len() > self.max_message_size {
            return false;
        }

        if !self.allowed_priorities.contains(&message.priority) {
            return false;
        }

        if self.blocked_senders.contains(&message.sender) {
            return false;
        }

        true
    }
}

src/sandbox/permissions/mod.rs
pub mod audit;

pub use audit::*;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct PermissionManager {
    policies: Arc<RwLock<HashMap<u32, ProcessPermissions>>>,
    auditor: SecurityAuditor,
    capability_checker: CapabilityChecker,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessPermissions {
    pub process_id: u32,
    pub capabilities: HashSet<Capability>,
    pub resource_limits: ResourceLimits,
    pub network_access: NetworkAccess,
    pub file_access: FileAccess,
    pub ipc_permissions: IpcPermissions,
    pub granted_at: std::time::SystemTime,
    pub expires_at: Option<std::time::SystemTime>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    ReadFiles(String),
    WriteFiles(String),
    NetworkRequest(String),
    CreateProcess,
    ManageMemory,
    AccessGpu,
    PlayAudio,
    CaptureVideo,
    AccessCamera,
    AccessMicrophone,
    AccessSerialPort,
    AccessUsb,
    ExecuteScript,
    ModifyDom,
    CacheAccess,
    StorageAccess,
    NotificationAccess,
    GeolocationAccess,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub max_memory_bytes: u64,
    pub max_cpu_percent: u8,
    pub max_file_descriptors: u32,
    pub max_network_connections: u32,
    pub max_execution_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkAccess {
    pub allowed_domains: HashSet<String>,
    pub allowed_ports: HashSet<u16>,
    pub blocked_ips: HashSet<std::net::IpAddr>,
    pub max_bandwidth_bps: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileAccess {
    pub readable_paths: HashSet<String>,
    pub writable_paths: HashSet<String>,
    pub executable_paths: HashSet<String>,
    pub temp_directory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcPermissions {
    pub can_send_to: HashSet<u32>,
    pub can_receive_from: HashSet<u32>,
    pub allowed_message_types: HashSet<String>,
    pub max_message_size: usize,
}

struct CapabilityChecker {
    system_capabilities: HashSet<Capability>,
    revoked_capabilities: HashMap<u32, HashSet<Capability>>,
}

impl PermissionManager {
    pub fn new() -> Self {
        Self {
            policies: Arc::new(RwLock::new(HashMap::new())),
            auditor: SecurityAuditor::new(),
            capability_checker: CapabilityChecker::new(),
        }
    }

    pub async fn grant_permissions(&self, process_id: u32, permissions: ProcessPermissions) -> Result<(), PermissionError> {
        if !self.capability_checker.validate_capabilities(&permissions.capabilities).await {
            return Err(PermissionError::InvalidCapability);
        }

        {
            let mut policies = self.policies.write().await;
            policies.insert(process_id, permissions.clone());
        }

        self.auditor.log_permission_grant(process_id, &permissions).await;
        
        Ok(())
    }

    pub async fn revoke_permission(&self, process_id: u32, capability: Capability) -> Result<(), PermissionError> {
        {
            let mut policies = self.policies.write().await;
            if let Some(permissions) = policies.get_mut(&process_id) {
                permissions.capabilities.remove(&capability);
            } else {
                return Err(PermissionError::ProcessNotFound);
            }
        }

        self.auditor.log_permission_revoke(process_id, &capability).await;
        Ok(())
    }

    pub async fn check_permission(&self, process_id: u32, capability: &Capability) -> Result<bool, PermissionError> {
        let policies = self.policies.read().await;
        if let Some(permissions) = policies.get(&process_id) {
            if let Some(expires_at) = permissions.expires_at {
                if std::time::SystemTime::now() > expires_at {
                    return Ok(false);
                }
            }

            let has_permission = permissions.capabilities.contains(capability);
            
            if has_permission {
                self.auditor.log_permission_check(process_id, capability, true).await;
            }
            
            Ok(has_permission)
        } else {
            Err(PermissionError::ProcessNotFound)
        }
    }

    pub async fn can_communicate(&self, from: u32, to: u32) -> Result<bool, PermissionError> {
        let policies = self.policies.read().await;
        if let Some(from_permissions) = policies.get(&from) {
            Ok(from_permissions.ipc_permissions.can_send_to.contains(&to))
        } else {
            Ok(false)
        }
    }

    pub async fn check_resource_usage(&self, process_id: u32, usage: &ResourceUsage) -> Result<bool, PermissionError> {
        let policies = self.policies.read().await;
        if let Some(permissions) = policies.get(&process_id) {
            let within_limits = 
                usage.memory_bytes <= permissions.resource_limits.max_memory_bytes &&
                usage.cpu_percent <= permissions.resource_limits.max_cpu_percent &&
                usage.file_descriptors <= permissions.resource_limits.max_file_descriptors &&
                usage.network_connections <= permissions.resource_limits.max_network_connections;

            if !within_limits {
                self.auditor.log_resource_violation(process_id, usage, &permissions.resource_limits).await;
            }

            Ok(within_limits)
        } else {
            Err(PermissionError::ProcessNotFound)
        }
    }

    pub async fn cleanup_expired_permissions(&self) -> Result<Vec<u32>, PermissionError> {
        let now = std::time::SystemTime::now();
        let mut expired_processes = Vec::new();
        
        {
            let mut policies = self.policies.write().await;
            policies.retain(|&process_id, permissions| {
                if let Some(expires_at) = permissions.expires_at {
                    if now > expires_at {
                        expired_processes.push(process_id);
                        return false;
                    }
                }
                true
            });
        }

        for process_id in &expired_processes {
            self.auditor.log_permission_expiry(*process_id).await;
        }

        Ok(expired_processes)
    }

    pub async fn get_process_permissions(&self, process_id: u32) -> Option<ProcessPermissions> {
        let policies = self.policies.read().await;
        policies.get(&process_id).cloned()
    }

    pub async fn update_resource_limits(&self, process_id: u32, new_limits: ResourceLimits) -> Result<(), PermissionError> {
        {
            let mut policies = self.policies.write().await;
            if let Some(permissions) = policies.get_mut(&process_id) {
                permissions.resource_limits = new_limits.clone();
            } else {
                return Err(PermissionError::ProcessNotFound);
            }
        }

        self.auditor.log_resource_limit_update(process_id, &new_limits).await;
        Ok(())
    }

    pub async fn get_security_report(&self) -> SecurityReport {
        self.auditor.generate_security_report().await
    }
}

impl CapabilityChecker {
    fn new() -> Self {
        let mut system_capabilities = HashSet::new();
        system_capabilities.insert(Capability::ReadFiles("/tmp/*".to_string()));
        system_capabilities.insert(Capability::NetworkRequest("https://*".to_string()));
        system_capabilities.insert(Capability::ExecuteScript);
        system_capabilities.insert(Capability::ModifyDom);
        system_capabilities.insert(Capability::CacheAccess);
        system_capabilities.insert(Capability::StorageAccess);

        Self {
            system_capabilities,
            revoked_capabilities: HashMap::new(),
        }
    }

    async fn validate_capabilities(&self, capabilities: &HashSet<Capability>) -> bool {
        for capability in capabilities {
            if !self.is_capability_available(capability) {
                return false;
            }
        }
        true
    }

    fn is_capability_available(&self, capability: &Capability) -> bool {
        match capability {
            Capability::AccessSerialPort | Capability::AccessUsb => {
                cfg!(target_os = "linux") || cfg!(target_os = "windows")
            }
            Capability::AccessCamera | Capability::AccessMicrophone => {
                true
            }
            _ => self.system_capabilities.contains(capability)
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResourceUsage {
    pub memory_bytes: u64,
    pub cpu_percent: u8,
    pub file_descriptors: u32,
    pub network_connections: u32,
    pub execution_time_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum PermissionError {
    #[error("Invalid capability")]
    InvalidCapability,
    #[error("Process not found")]
    ProcessNotFound,
    #[error("Permission denied")]
    PermissionDenied,
    #[error("Resource limit exceeded")]
    ResourceLimitExceeded,
    #[error("Capability not available on this system")]
    CapabilityNotAvailable,
}

impl Default for PermissionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory_bytes: 512 * 1024 * 1024,
            max_cpu_percent: 80,
            max_file_descriptors: 1024,
            max_network_connections: 100,
            max_execution_time_ms: 30000,
        }
    }
}

impl Default for NetworkAccess {
    fn default() -> Self {
        Self {
            allowed_domains: HashSet::new(),
            allowed_ports: [80, 443, 8080].iter().cloned().collect(),
            blocked_ips: HashSet::new(),
            max_bandwidth_bps: 10 * 1024 * 1024,
        }
    }
}

impl Default for FileAccess {
    fn default() -> Self {
        Self {
            readable_paths: ["/tmp/*".to_string()].iter().cloned().collect(),
            writable_paths: ["/tmp/*".to_string()].iter().cloned().collect(),
            executable_paths: HashSet::new(),
            temp_directory: Some("/tmp".to_string()),
        }
    }
}

impl Default for IpcPermissions {
    fn default() -> Self {
        Self {
            can_send_to: HashSet::new(),
            can_receive_from: HashSet::new(),
            allowed_message_types: ["RenderCommand", "DomUpdate"].iter().map(|s| s.to_string()).collect(),
            max_message_size: 1024 * 1024,
        }
    }
}

src/sandbox/permissions/audit.rs
use super::{Capability, ProcessPermissions, ResourceLimits, ResourceUsage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;

pub struct SecurityAuditor {
    audit_log: Arc<RwLock<Vec<AuditEvent>>>,
    file_writer: Arc<tokio::sync::Mutex<Option<tokio::fs::File>>>,
    metrics: Arc<RwLock<SecurityMetrics>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub timestamp: u64,
    pub process_id: u32,
    pub event_type: AuditEventType,
    pub severity: SeverityLevel,
    pub details: String,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditEventType {
    PermissionGrant,
    PermissionRevoke,
    PermissionCheck,
    PermissionDenied,
    ResourceViolation,
    SecurityViolation,
    ProcessStart,
    ProcessTerminate,
    NetworkAccess,
    FileAccess,
    IpcCommunication,
    PermissionExpiry,
    ResourceLimitUpdate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum SeverityLevel {
    Critical = 0,
    High = 1,
    Medium = 2,
    Low = 3,
    Info = 4,
}

#[derive(Debug, Default, Clone)]
pub struct SecurityMetrics {
    pub total_events: u64,
    pub security_violations: u64,
    pub permission_denials: u64,
    pub resource_violations: u64,
    pub active_processes: u32,
    pub high_risk_processes: u32,
    pub events_by_severity: HashMap<SeverityLevel, u64>,
    pub events_by_process: HashMap<u32, u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecurityReport {
    pub report_timestamp: u64,
    pub metrics: SecurityMetrics,
    pub recent_violations: Vec<AuditEvent>,
    pub risk_assessment: RiskAssessment,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RiskAssessment {
    pub overall_risk_level: RiskLevel,
    pub process_risk_scores: HashMap<u32, f64>,
    pub threat_indicators: Vec<ThreatIndicator>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreatIndicator {
    pub indicator_type: String,
    pub description: String,
    pub confidence: f64,
    pub first_seen: u64,
    pub last_seen: u64,
    pub count: u32,
}

impl SecurityAuditor {
    pub fn new() -> Self {
        let auditor = Self {
            audit_log: Arc::new(RwLock::new(Vec::new())),
            file_writer: Arc::new(tokio::sync::Mutex::new(None)),
            metrics: Arc::new(RwLock::new(SecurityMetrics::default())),
        };

        auditor.init_file_writer();
        auditor.start_metrics_aggregator();
        auditor
    }

    fn init_file_writer(&self) {
        let file_writer = self.file_writer.clone();
        tokio::spawn(async move {
            match OpenOptions::new()
                .create(true)
                .append(true)
                .open("security_audit.log")
                .await
            {
                Ok(file) => {
                    *file_writer.lock().await = Some(file);
                }
                Err(e) => {
                    log::error!("Failed to open audit log file: {}", e);
                }
            }
        });
    }

    fn start_metrics_aggregator(&self) {
        let metrics = self.metrics.clone();
        let audit_log = self.audit_log.clone();
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            
            loop {
                interval.tick().await;
                Self::aggregate_metrics(metrics.clone(), audit_log.clone()).await;
            }
        });
    }

    async fn aggregate_metrics(
        metrics: Arc<RwLock<SecurityMetrics>>,
        audit_log: Arc<RwLock<Vec<AuditEvent>>>,
    ) {
        let events = audit_log.read().await;
        let mut metrics_guard = metrics.write().await;
        
        metrics_guard.total_events = events.len() as u64;
        metrics_guard.events_by_severity.clear();
        metrics_guard.events_by_process.clear();
        
        for event in events.iter() {
            *metrics_guard.events_by_severity.entry(event.severity).or_insert(0) += 1;
            *metrics_guard.events_by_process.entry(event.process_id).or_insert(0) += 1;
            
            match event.event_type {
                AuditEventType::SecurityViolation => metrics_guard.security_violations += 1,
                AuditEventType::PermissionDenied => metrics_guard.permission_denials += 1,
                AuditEventType::ResourceViolation => metrics_guard.resource_violations += 1,
                _ => {}
            }
        }
    }

    pub async fn log_permission_grant(&self, process_id: u32, permissions: &ProcessPermissions) {
        let capabilities_str = permissions.capabilities
            .iter()
            .map(|c| format!("{:?}", c))
            .collect::<Vec<_>>()
            .join(", ");

        let event = AuditEvent {
            timestamp: self.current_timestamp(),
            process_id,
            event_type: AuditEventType::PermissionGrant,
            severity: SeverityLevel::Info,
            details: format!("Granted capabilities: {}", capabilities_str),
            metadata: self.create_metadata(&[
                ("capabilities_count", &permissions.capabilities.len().to_string()),
                ("memory_limit", &permissions.resource_limits.max_memory_bytes.to_string()),
            ]),
        };

        self.record_event(event).await;
    }

    pub async fn log_permission_revoke(&self, process_id: u32, capability: &Capability) {
        let event = AuditEvent {
            timestamp: self.current_timestamp(),
            process_id,
            event_type: AuditEventType::PermissionRevoke,
            severity: SeverityLevel::Medium,
            details: format!("Revoked capability: {:?}", capability),
            metadata: self.create_metadata(&[("capability", &format!("{:?}", capability))]),
        };

        self.record_event(event).await;
    }

    pub async fn log_permission_check(&self, process_id: u32, capability: &Capability, granted: bool) {
        let event = AuditEvent {
            timestamp: self.current_timestamp(),
            process_id,
            event_type: if granted { AuditEventType::PermissionCheck } else { AuditEventType::PermissionDenied },
            severity: if granted { SeverityLevel::Low } else { SeverityLevel::Medium },
            details: format!("Permission check for {:?}: {}", capability, if granted { "GRANTED" } else { "DENIED" }),
            metadata: self.create_metadata(&[
                ("capability", &format!("{:?}", capability)),
                ("result", if granted { "granted" } else { "denied" }),
            ]),
        };

        self.record_event(event).await;
    }

    pub async fn log_resource_violation(&self, process_id: u32, usage: &ResourceUsage, limits: &ResourceLimits) {
        let violations = self.identify_violations(usage, limits);
        
        let event = AuditEvent {
            timestamp: self.current_timestamp(),
            process_id,
            event_type: AuditEventType::ResourceViolation,
            severity: SeverityLevel::High,
            details: format!("Resource limit violations: {}", violations.join(", ")),
            metadata: self.create_metadata(&[
                ("memory_usage", &usage.memory_bytes.to_string()),
                ("memory_limit", &limits.max_memory_bytes.to_string()),
                ("cpu_usage", &usage.cpu_percent.to_string()),
                ("cpu_limit", &limits.max_cpu_percent.to_string()),
            ]),
        };

        self.record_event(event).await;
    }

    pub async fn log_security_violation(&self, process_id: u32, violation_type: &str, details: &str) {
        let event = AuditEvent {
            timestamp: self.current_timestamp(),
            process_id,
            event_type: AuditEventType::SecurityViolation,
            severity: SeverityLevel::Critical,
            details: format!("Security violation: {} - {}", violation_type, details),
            metadata: self.create_metadata(&[("violation_type", violation_type)]),
        };

        self.record_event(event).await;
    }

    pub async fn log_permission_expiry(&self, process_id: u32) {
        let event = AuditEvent {
            timestamp: self.current_timestamp(),
            process_id,
            event_type: AuditEventType::PermissionExpiry,
            severity: SeverityLevel::Info,
            details: "Process permissions expired".to_string(),
            metadata: HashMap::new(),
        };

        self.record_event(event).await;
    }

    pub async fn log_resource_limit_update(&self, process_id: u32, new_limits: &ResourceLimits) {
        let event = AuditEvent {
            timestamp: self.current_timestamp(),
            process_id,
            event_type: AuditEventType::ResourceLimitUpdate,
            severity: SeverityLevel::Medium,
            details: "Resource limits updated".to_string(),
            metadata: self.create_metadata(&[
                ("memory_limit", &new_limits.max_memory_bytes.to_string()),
                ("cpu_limit", &new_limits.max_cpu_percent.to_string()),
            ]),
        };

        self.record_event(event).await;
    }

    async fn record_event(&self, event: AuditEvent) {
        {
            let mut log = self.audit_log.write().await;
            log.push(event.clone());
            
            if log.len() > 10000 {
                log.drain(0..1000);
            }
        }

        self.write_to_file(&event).await;
        
        {
            let mut metrics = self.metrics.write().await;
            metrics.total_events += 1;
            *metrics.events_by_severity.entry(event.severity).or_insert(0) += 1;
            *metrics.events_by_process.entry(event.process_id).or_insert(0) += 1;
        }
    }

    async fn write_to_file(&self, event: &AuditEvent) {
        if let Some(file) = self.file_writer.lock().await.as_mut() {
            let json_line = serde_json::to_string(event)
                .unwrap_or_else(|_| "SERIALIZATION_ERROR".to_string());
            
            if let Err(e) = file.write_all(format!("{}\n", json_line).as_bytes()).await {
                log::error!("Failed to write audit event to file: {}", e);
            }
        }
    }

    pub async fn generate_security_report(&self) -> SecurityReport {
        let metrics = self.metrics.read().await.clone();
        let recent_violations = self.get_recent_violations().await;
        let risk_assessment = self.assess_risk().await;
        let recommendations = self.generate_recommendations(&metrics, &risk_assessment);

        SecurityReport {
            report_timestamp: self.current_timestamp(),
            metrics,
            recent_violations,
            risk_assessment,
            recommendations,
        }
    }

    async fn get_recent_violations(&self) -> Vec<AuditEvent> {
        let log = self.audit_log.read().await;
        let cutoff_time = self.current_timestamp() - (24 * 60 * 60 * 1000);
        
        log.iter()
            .filter(|event| {
                event.timestamp > cutoff_time && 
                matches!(event.event_type, 
                    AuditEventType::SecurityViolation | 
                    AuditEventType::ResourceViolation |
                    AuditEventType::PermissionDenied
                )
            })
            .cloned()
            .collect()
    }

    async fn assess_risk(&self) -> RiskAssessment {
        let metrics = self.metrics.read().await;
        let log = self.audit_log.read().await;
        
        let mut process_risk_scores = HashMap::new();
        let mut threat_indicators = Vec::new();
        
        for (process_id, event_count) in &metrics.events_by_process {
            let violation_count = log.iter()
                .filter(|e| e.process_id == *process_id && 
                    matches!(e.event_type, AuditEventType::SecurityViolation | AuditEventType::ResourceViolation))
                .count();
            
            let risk_score = (violation_count as f64 / *event_count as f64) * 100.0;
            process_risk_scores.insert(*process_id, risk_score);
            
            if risk_score > 20.0 {
                threat_indicators.push(ThreatIndicator {
                    indicator_type: "High violation rate".to_string(),
                    description: format!("Process {} has {}% violation rate", process_id, risk_score),
                    confidence: risk_score / 100.0,
                    first_seen: self.current_timestamp() - 86400000,
                    last_seen: self.current_timestamp(),
                    count: violation_count as u32,
                });
            }
        }

        let overall_risk_level = if metrics.security_violations > 10 {
            RiskLevel::Critical
        } else if metrics.security_violations > 5 {
            RiskLevel::High
        } else if metrics.resource_violations > 20 {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        };

        RiskAssessment {
            overall_risk_level,
            process_risk_scores,
            threat_indicators,
        }
    }

    fn generate_recommendations(&self, metrics: &SecurityMetrics, risk: &RiskAssessment) -> Vec<String> {
        let mut recommendations = Vec::new();

        if metrics.security_violations > 5 {
            recommendations.push("Review and strengthen security policies".to_string());
        }

        if metrics.resource_violations > 10 {
            recommendations.push("Adjust resource limits for high-usage processes".to_string());
        }

        if risk.overall_risk_level >= RiskLevel::High {
            recommendations.push("Immediate security review required".to_string());
        }

        for (process_id, score) in &risk.process_risk_scores {
            if *score > 30.0 {
                recommendations.push(format!("Investigate process {} for potential security issues", process_id));
            }
        }

        if recommendations.is_empty() {
            recommendations.push("Security posture is good, continue monitoring".to_string());
        }

        recommendations
    }

    fn identify_violations(&self, usage: &ResourceUsage, limits: &ResourceLimits) -> Vec<String> {
        let mut violations = Vec::new();

        if usage.memory_bytes > limits.max_memory_bytes {
            violations.push(format!("Memory: {} > {}", usage.memory_bytes, limits.max_memory_bytes));
        }

        if usage.cpu_percent > limits.max_cpu_percent {
            violations.push(format!("CPU: {}% > {}%", usage.cpu_percent, limits.max_cpu_percent));
        }

        if usage.file_descriptors > limits.max_file_descriptors {
            violations.push(format!("File descriptors: {} > {}", usage.file_descriptors, limits.max_file_descriptors));
        }

        violations
    }

    fn current_timestamp(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn create_metadata(&self, pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    pub async fn get_audit_events(&self, process_id: Option<u32>, limit: Option<usize>) -> Vec<AuditEvent> {
        let log = self.audit_log.read().await;
        let filtered: Vec<AuditEvent> = log.iter()
            .filter(|event| process_id.map_or(true, |pid| event.process_id == pid))
            .rev()
            .take(limit.unwrap_or(100))
            .cloned()
            .collect();
        
        filtered
    }
}

impl Default for SecurityAuditor {
    fn default() -> Self {
        Self::new()
    }
}