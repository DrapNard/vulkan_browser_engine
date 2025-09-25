pub mod ipc;
pub mod permissions;
pub mod process;
pub mod security;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

pub type ProcessId = u32;

static NEXT_PROCESS_ID: AtomicU32 = AtomicU32::new(1);

#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    pub max_processes: u32,
    pub max_memory_per_process: u64,
    pub allowed_syscalls: Vec<String>,
    pub network_isolation: bool,
    pub file_system_restrictions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SecurityAuditReport {
    pub timestamp: u64,
    pub total_processes: usize,
    pub security_violations: Vec<SecurityViolation>,
    pub resource_usage: ResourceUsageReport,
    pub compliance_status: ComplianceStatus,
}

#[derive(Debug, Clone)]
pub struct SecurityViolation {
    pub process_id: ProcessId,
    pub violation_type: ViolationType,
    pub severity: Severity,
    pub description: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone)]
pub enum ViolationType {
    UnauthorizedSyscall,
    ExcessiveMemoryUsage,
    UnauthorizedNetworkAccess,
    FileSystemViolation,
    IpcViolation,
}

#[derive(Debug, Clone)]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone)]
pub struct ResourceUsageReport {
    pub total_memory_used: u64,
    pub total_cpu_usage: f64,
    pub network_connections: u32,
    pub file_descriptors: u32,
    pub total_threads: u32,
}

#[derive(Debug, Clone)]
pub enum ComplianceStatus {
    Compliant,
    NonCompliant(Vec<String>),
    PartiallyCompliant(Vec<String>),
}

pub struct SandboxManager {
    processes: Arc<RwLock<HashMap<ProcessId, process::SandboxedProcess>>>,
    permission_manager: Arc<permissions::PermissionManager>,
    security_policy: SecurityPolicy,
    ipc_manager: Arc<ipc::IpcManager>,
    max_processes: u32,
}

impl SandboxManager {
    pub async fn new() -> Result<Self, SandboxError> {
        Self::with_config(SandboxConfig::default()).await
    }

    pub async fn with_config(config: SandboxConfig) -> Result<Self, SandboxError> {
        let permission_manager = Arc::new(permissions::PermissionManager::new().await?);
        let ipc_manager = Arc::new(ipc::IpcManager::new());

        Ok(Self {
            processes: Arc::new(RwLock::new(HashMap::with_capacity(config.initial_capacity))),
            permission_manager,
            security_policy: config.security_policy,
            ipc_manager,
            max_processes: config.max_processes,
        })
    }

    pub async fn create_sandboxed_process(
        &self,
        config: process::ProcessConfig,
    ) -> Result<ProcessId, SandboxError> {
        self.validate_process_creation(&config).await?;

        let process_id = Self::generate_process_id();
        let sandboxed_process = process::SandboxedProcess::new(process_id, config).await?;

        {
            let mut processes = self.processes.write().await;
            if processes.len() >= self.max_processes as usize {
                return Err(SandboxError::ResourceExhausted(
                    "Maximum process count reached".into(),
                ));
            }
            processes.insert(process_id, sandboxed_process);
        }

        info!("Created sandboxed process: {}", process_id);
        Ok(process_id)
    }

    pub async fn terminate_process(&self, process_id: ProcessId) -> Result<(), SandboxError> {
        let mut process = {
            let mut processes = self.processes.write().await;
            processes
                .remove(&process_id)
                .ok_or(SandboxError::ProcessNotFound(process_id))?
        };

        match process.terminate().await {
            Ok(_) => {
                info!("Successfully terminated process: {}", process_id);
                Ok(())
            }
            Err(e) => {
                error!("Failed to terminate process {}: {:?}", process_id, e);
                Err(SandboxError::ProcessError(e))
            }
        }
    }

    pub async fn send_message(
        &self,
        from: ProcessId,
        to: ProcessId,
        message: ipc::IpcMessage,
    ) -> Result<(), SandboxError> {
        self.validate_process_exists(from).await?;
        self.validate_process_exists(to).await?;

        if !self.permission_manager.can_communicate(from, to).await? {
            warn!("IPC communication denied: {} -> {}", from, to);
            return Err(SandboxError::PermissionDenied(format!(
                "IPC communication not allowed from {} to {}",
                from, to
            )));
        }

        self.ipc_manager
            .send_message(from, to, message)
            .await
            .map_err(SandboxError::IpcError)
    }

    pub async fn audit_security(&self) -> Result<SecurityAuditReport, SandboxError> {
        let processes = self.processes.read().await;
        let mut violations = Vec::new();
        let mut total_memory = 0u64;
        let mut total_cpu_usage = 0f64;
        let mut total_threads = 0u32;
        let mut total_file_handles = 0u32;

        for (process_id, process) in processes.iter() {
            let stats = process.get_stats().await;
            total_memory += stats.memory_usage_bytes;
            total_cpu_usage += stats.cpu_usage_percent;
            total_threads += stats.thread_count;
            total_file_handles += stats.file_handles_count;

            let process_violations = self.check_process_violations(*process_id, &stats).await;
            violations.extend(process_violations);
        }

        let compliance_status = if violations.is_empty() {
            ComplianceStatus::Compliant
        } else {
            let critical_violations: Vec<String> = violations
                .iter()
                .filter(|v| matches!(v.severity, Severity::Critical))
                .map(|v| v.description.clone())
                .collect();

            if critical_violations.is_empty() {
                ComplianceStatus::PartiallyCompliant(
                    violations.iter().map(|v| v.description.clone()).collect(),
                )
            } else {
                ComplianceStatus::NonCompliant(critical_violations)
            }
        };

        Ok(SecurityAuditReport {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            total_processes: processes.len(),
            security_violations: violations,
            resource_usage: ResourceUsageReport {
                total_memory_used: total_memory,
                total_cpu_usage,
                network_connections: 0,
                file_descriptors: total_file_handles,
                total_threads,
            },
            compliance_status,
        })
    }

    pub async fn get_process_stats(&self) -> Vec<process::ProcessStats> {
        let processes = self.processes.read().await;
        let mut stats = Vec::with_capacity(processes.len());

        for process in processes.values() {
            stats.push(process.get_stats().await);
        }

        stats
    }

    pub async fn get_process_count(&self) -> usize {
        self.processes.read().await.len()
    }

    pub async fn force_cleanup(&self) -> Result<(), SandboxError> {
        let process_ids: Vec<ProcessId> = {
            let processes = self.processes.read().await;
            processes.keys().copied().collect()
        };

        for process_id in process_ids {
            if let Err(e) = self.terminate_process(process_id).await {
                warn!("Failed to cleanup process {}: {:?}", process_id, e);
            }
        }

        Ok(())
    }

    pub async fn monitor_resource_usage(&self) -> Result<ResourceUsageReport, SandboxError> {
        let processes = self.processes.read().await;
        let mut total_memory = 0u64;
        let mut total_cpu_usage = 0f64;
        let mut total_threads = 0u32;
        let mut total_file_handles = 0u32;

        for process in processes.values() {
            let stats = process.get_stats().await;
            total_memory += stats.memory_usage_bytes;
            total_cpu_usage += stats.cpu_usage_percent;
            total_threads += stats.thread_count;
            total_file_handles += stats.file_handles_count;
        }

        Ok(ResourceUsageReport {
            total_memory_used: total_memory,
            total_cpu_usage,
            network_connections: 0,
            file_descriptors: total_file_handles,
            total_threads,
        })
    }

    fn generate_process_id() -> ProcessId {
        NEXT_PROCESS_ID.fetch_add(1, Ordering::SeqCst)
    }

    async fn validate_process_creation(
        &self,
        _config: &process::ProcessConfig,
    ) -> Result<(), SandboxError> {
        let current_count = self.processes.read().await.len();
        if current_count >= self.max_processes as usize {
            return Err(SandboxError::ResourceExhausted(
                "Process limit reached".into(),
            ));
        }

        Ok(())
    }

    async fn validate_process_exists(&self, process_id: ProcessId) -> Result<(), SandboxError> {
        let processes = self.processes.read().await;
        if !processes.contains_key(&process_id) {
            return Err(SandboxError::ProcessNotFound(process_id));
        }
        Ok(())
    }

    async fn check_process_violations(
        &self,
        process_id: ProcessId,
        stats: &process::ProcessStats,
    ) -> Vec<SecurityViolation> {
        let mut violations = Vec::new();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        if stats.memory_usage_bytes > self.security_policy.max_memory_per_process {
            violations.push(SecurityViolation {
                process_id,
                violation_type: ViolationType::ExcessiveMemoryUsage,
                severity: Severity::High,
                description: format!(
                    "Process {} exceeds memory limit: {} > {}",
                    process_id,
                    stats.memory_usage_bytes,
                    self.security_policy.max_memory_per_process
                ),
                timestamp,
            });
        }

        if stats.cpu_usage_percent > 95.0 {
            violations.push(SecurityViolation {
                process_id,
                violation_type: ViolationType::UnauthorizedSyscall,
                severity: Severity::Medium,
                description: format!(
                    "Process {} shows excessive CPU usage: {:.2}%",
                    process_id, stats.cpu_usage_percent
                ),
                timestamp,
            });
        }

        if stats.file_handles_count > 1000 {
            violations.push(SecurityViolation {
                process_id,
                violation_type: ViolationType::FileSystemViolation,
                severity: Severity::Medium,
                description: format!(
                    "Process {} has excessive file handles: {}",
                    process_id, stats.file_handles_count
                ),
                timestamp,
            });
        }

        violations
    }
}

#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub max_processes: u32,
    pub initial_capacity: usize,
    pub security_policy: SecurityPolicy,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            max_processes: 1000,
            initial_capacity: 64,
            security_policy: SecurityPolicy::default(),
        }
    }
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            max_processes: 1000,
            max_memory_per_process: 1024 * 1024 * 1024,
            allowed_syscalls: vec![
                "read".to_string(),
                "write".to_string(),
                "open".to_string(),
                "close".to_string(),
                "mmap".to_string(),
                "munmap".to_string(),
                "brk".to_string(),
                "exit".to_string(),
            ],
            network_isolation: true,
            file_system_restrictions: vec![
                "/tmp".to_string(),
                "/var/tmp".to_string(),
                "/proc/self".to_string(),
            ],
        }
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

    #[error("Permission error: {0}")]
    PermissionError(#[from] permissions::PermissionError),

    #[error("Security violation: {0}")]
    SecurityViolation(String),

    #[error("Process not found: {0}")]
    ProcessNotFound(ProcessId),

    #[error("Resource exhausted: {0}")]
    ResourceExhausted(String),

    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    #[error("System error: {0}")]
    SystemError(String),
}

impl Default for SandboxManager {
    fn default() -> Self {
        tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(Self::new()))
            .expect("Failed to create default SandboxManager")
    }
}
