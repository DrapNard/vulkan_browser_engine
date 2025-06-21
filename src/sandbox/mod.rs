pub mod ipc;
pub mod permissions;
pub mod process;
pub mod security;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct SandboxManager {
    processes: Arc<RwLock<HashMap<ProcessId, process::SandboxedProcess>>>,
    permission_manager: permissions::PermissionManager,
    security_policy: security::SecurityPolicy,
    ipc_manager: ipc::IpcManager,
}

pub type ProcessId = u32;

impl SandboxManager {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(RwLock::new(HashMap::new())),
            permission_manager: permissions::PermissionManager::new(),
            security_policy: security::SecurityPolicy::default(),
            ipc_manager: ipc::IpcManager::new(),
        }
    }

    pub async fn create_sandboxed_process(&self, config: process::ProcessConfig) -> Result<ProcessId, SandboxError> {
        let process_id = self.generate_process_id();
        let process = process::SandboxedProcess::new(process_id, config).await?;
        
        {
            let mut processes = self.processes.write().await;
            processes.insert(process_id, process);
        }

        log::info!("Created sandboxed process: {}", process_id);
        Ok(process_id)
    }

    pub async fn terminate_process(&self, process_id: ProcessId) -> Result<(), SandboxError> {
        let mut processes = self.processes.write().await;
        if let Some(process) = processes.remove(&process_id) {
            process.terminate().await?;
            log::info!("Terminated sandboxed process: {}", process_id);
        }
        Ok(())
    }

    pub async fn send_message(&self, from: ProcessId, to: ProcessId, message: ipc::IpcMessage) -> Result<(), SandboxError> {
        if !self.permission_manager.can_communicate(from, to).await? {
            return Err(SandboxError::PermissionDenied("IPC communication not allowed".to_string()));
        }

        self.ipc_manager.send_message(from, to, message).await
    }

    fn generate_process_id(&self) -> ProcessId {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT_ID: AtomicU32 = AtomicU32::new(1);
        NEXT_ID.fetch_add(1, Ordering::SeqCst)
    }

    pub async fn audit_security(&self) -> security::SecurityAuditReport {
        self.security_policy.create_audit_report().await
    }

    pub async fn get_process_stats(&self) -> Vec<process::ProcessStats> {
        let processes = self.processes.read().await;
        let mut stats = Vec::new();
        
        for process in processes.values() {
            stats.push(process.get_stats().await);
        }
        
        stats
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Process error: {0}")]
    ProcessError(#[from] process::ProcessError),
    #[error("IPC error: {0}")]
    IpcError(#[from] ipc::IpcError),
    #[error("Security violation: {0}")]
    SecurityViolation(String),
}

impl Default for SandboxManager {
    fn default() -> Self {
        Self::new()
    }
}