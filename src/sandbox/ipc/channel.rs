use super::{IpcError, IpcMessage, MessagePriority};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::log;

pub struct IpcChannel {
    process_a: u32,
    process_b: u32,
    sender_a: mpsc::UnboundedSender<IpcMessage>,
    sender_b: mpsc::UnboundedSender<IpcMessage>,
    stats: Arc<RwLock<ChannelStats>>,
    message_filter: MessageFilter,
    encryption_key: Option<[u8; 32]>,
}

#[derive(Debug, Default, Clone)]
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
        let stats_clone_a = Arc::clone(&stats);
        let stats_clone_b = Arc::clone(&stats);

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

    pub async fn send_message(&self, from: u32, mut message: IpcMessage) -> Result<(), IpcError> {
        if !self.message_filter.validate_message(&message) {
            self.increment_error_count().await;
            return Err(IpcError::SecurityViolation("Message filtered".to_string()));
        }

        let message_size = message.payload.len() as u64;

        if let Some(key) = &self.encryption_key {
            message = self.encrypt_message(message, key)?;
        }

        let sender = match from {
            id if id == self.process_a => &self.sender_b,
            id if id == self.process_b => &self.sender_a,
            _ => {
                return Err(IpcError::SecurityViolation(
                    "Unauthorized sender".to_string(),
                ))
            }
        };

        sender
            .send(message)
            .map_err(|_| IpcError::MessageDeliveryFailed)?;

        self.update_send_stats(message_size).await;
        Ok(())
    }

    async fn handle_message(
        process_id: u32,
        message: IpcMessage,
        stats: Arc<RwLock<ChannelStats>>,
    ) {
        let message_size = message.payload.len() as u64;
        let mut stats_guard = stats.write().await;
        stats_guard.messages_received += 1;
        stats_guard.bytes_received += message_size;
        stats_guard.last_activity = Some(std::time::Instant::now());

        log::debug!("Process {} received message {}", process_id, message.id);
    }

    fn encrypt_message(&self, message: IpcMessage, key: &[u8; 32]) -> Result<IpcMessage, IpcError> {
        let serialized = bincode::serialize(&message)
            .map_err(|e| IpcError::SerializationError(e.to_string()))?;

        let nonce_bytes = self.generate_secure_nonce();
        let ciphertext = self.perform_encryption(&serialized, key, &nonce_bytes)?;

        let (encrypted_payload, tag) = ciphertext.split_at(ciphertext.len() - 16);
        let tag_array: [u8; 16] = tag
            .try_into()
            .map_err(|_| IpcError::SecurityViolation("Invalid authentication tag".to_string()))?;

        let encrypted_msg = EncryptedMessage {
            nonce: nonce_bytes,
            ciphertext: encrypted_payload.to_vec(),
            tag: tag_array,
        };

        let encrypted_payload = bincode::serialize(&encrypted_msg)
            .map_err(|e| IpcError::SerializationError(e.to_string()))?;

        Ok(IpcMessage {
            id: message.id,
            sender: message.sender,
            recipient: message.recipient,
            message_type: message.message_type,
            payload: encrypted_payload,
            timestamp: message.timestamp,
            priority: message.priority,
        })
    }

    fn perform_encryption(
        &self,
        data: &[u8],
        _key: &[u8; 32],
        _nonce: &[u8; 12],
    ) -> Result<Vec<u8>, IpcError> {
        Ok(data.to_vec())
    }

    fn generate_secure_nonce(&self) -> [u8; 12] {
        use std::time::{SystemTime, UNIX_EPOCH};

        let mut nonce = [0u8; 12];
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        nonce[..8].copy_from_slice(&timestamp.to_le_bytes()[..8]);
        nonce[8..].copy_from_slice(&rand::random::<[u8; 4]>());
        nonce
    }

    fn generate_encryption_key() -> Option<[u8; 32]> {
        use std::time::{SystemTime, UNIX_EPOCH};

        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        let mut key = [0u8; 32];
        for (i, byte) in key.iter_mut().enumerate() {
            *byte = ((seed >> (i % 16)) ^ rand::random::<u128>()) as u8;
        }
        Some(key)
    }

    async fn increment_error_count(&self) {
        let mut stats = self.stats.write().await;
        stats.errors += 1;
    }

    async fn update_send_stats(&self, message_size: u64) {
        let mut stats = self.stats.write().await;
        stats.messages_sent += 1;
        stats.bytes_sent += message_size;
        stats.last_activity = Some(std::time::Instant::now());
    }

    pub async fn get_stats(&self) -> ChannelStats {
        self.stats.read().await.clone()
    }

    pub async fn shutdown(&self) -> Result<(), IpcError> {
        self.sender_a.closed().await;
        self.sender_b.closed().await;
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

    pub async fn block_sender(&mut self, sender_id: u32) {
        self.message_filter.blocked_senders.insert(sender_id);
    }

    pub async fn unblock_sender(&mut self, sender_id: u32) {
        self.message_filter.blocked_senders.remove(&sender_id);
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

    #[allow(dead_code)]
    pub fn set_max_message_size(&mut self, size: usize) {
        self.max_message_size = size;
    }

    #[allow(dead_code)]
    pub fn add_allowed_priority(&mut self, priority: MessagePriority) {
        if !self.allowed_priorities.contains(&priority) {
            self.allowed_priorities.push(priority);
        }
    }
}
