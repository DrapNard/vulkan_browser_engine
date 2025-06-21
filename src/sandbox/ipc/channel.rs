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