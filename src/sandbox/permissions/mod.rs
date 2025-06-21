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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OptimizationLevel {
    None,
    Low,
    Medium,
    High,
    Custom(u8),
}