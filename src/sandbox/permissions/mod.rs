pub mod audit;

pub use audit::*;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::{Notify, RwLock};
use tokio::time::interval;
use tracing::{error, info};

pub struct PermissionManager {
    policies: Arc<RwLock<HashMap<u32, Arc<ProcessPermissions>>>>,
    capability_cache: Arc<RwLock<HashMap<(u32, Capability), (bool, SystemTime)>>>,
    auditor: Arc<SecurityAuditor>,
    capability_checker: Arc<CapabilityChecker>,
    metrics: Arc<PermissionMetrics>,
    cleanup_handle: Option<tokio::task::JoinHandle<()>>,
    shutdown_signal: Arc<Notify>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessPermissions {
    pub process_id: u32,
    pub capabilities: HashSet<Capability>,
    pub resource_limits: ResourceLimits,
    pub network_access: NetworkAccess,
    pub file_access: FileAccess,
    pub ipc_permissions: IpcPermissions,
    pub granted_at: SystemTime,
    pub expires_at: Option<SystemTime>,
    pub last_accessed: Arc<std::sync::atomic::AtomicU64>,
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
    capability_matrix: HashMap<Capability, bool>,
    platform_features: PlatformFeatures,
}

struct PlatformFeatures {
    has_camera: bool,
    has_microphone: bool,
    has_gpu: bool,
    has_serial_ports: bool,
    has_usb: bool,
}

struct PermissionMetrics {
    total_checks: AtomicU64,
    cache_hits: AtomicU64,
    permission_grants: AtomicU64,
    permission_denials: AtomicU64,
    expired_cleanups: AtomicU64,
}

impl PermissionManager {
    pub async fn new() -> Result<Self, PermissionError> {
        let auditor = Arc::new(SecurityAuditor::new().await);
        let capability_checker = Arc::new(CapabilityChecker::new().await);
        let shutdown_signal = Arc::new(Notify::new());

        let manager = Self {
            policies: Arc::new(RwLock::new(HashMap::new())),
            capability_cache: Arc::new(RwLock::new(HashMap::new())),
            auditor,
            capability_checker,
            metrics: Arc::new(PermissionMetrics::new()),
            cleanup_handle: None,
            shutdown_signal: Arc::clone(&shutdown_signal),
        };

        Ok(manager)
    }

    pub async fn start_background_tasks(&mut self) {
        let policies = Arc::clone(&self.policies);
        let cache = Arc::clone(&self.capability_cache);
        let auditor = Arc::clone(&self.auditor);
        let metrics = Arc::clone(&self.metrics);
        let shutdown = Arc::clone(&self.shutdown_signal);

        self.cleanup_handle = Some(tokio::spawn(async move {
            let mut cleanup_interval = interval(Duration::from_secs(30));
            let mut cache_cleanup_interval = interval(Duration::from_secs(300));

            loop {
                tokio::select! {
                    _ = cleanup_interval.tick() => {
                        Self::cleanup_expired_permissions_background(
                            Arc::clone(&policies),
                            Arc::clone(&auditor),
                            Arc::clone(&metrics)
                        ).await;
                    }
                    _ = cache_cleanup_interval.tick() => {
                        Self::cleanup_cache_background(Arc::clone(&cache)).await;
                    }
                    _ = shutdown.notified() => {
                        info!("Background cleanup tasks shutting down");
                        break;
                    }
                }
            }
        }));
    }

    pub async fn grant_permissions(
        &self,
        process_id: u32,
        mut permissions: ProcessPermissions,
    ) -> Result<(), PermissionError> {
        if !self
            .capability_checker
            .validate_capabilities_batch(&permissions.capabilities)
            .await
        {
            self.metrics
                .permission_denials
                .fetch_add(1, Ordering::Relaxed);
            return Err(PermissionError::InvalidCapability);
        }

        permissions.last_accessed = Arc::new(std::sync::atomic::AtomicU64::new(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        ));

        let permissions_arc = Arc::new(permissions.clone());
        self.policies
            .write()
            .await
            .insert(process_id, Arc::clone(&permissions_arc));
        self.invalidate_cache_for_process(process_id).await;

        self.auditor
            .log_permission_grant(process_id, &permissions)
            .await;
        self.metrics
            .permission_grants
            .fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    pub async fn revoke_permission(
        &self,
        process_id: u32,
        capability: Capability,
    ) -> Result<(), PermissionError> {
        let mut policies = self.policies.write().await;

        if let Some(permissions_arc) = policies.get(&process_id) {
            let mut new_permissions = (**permissions_arc).clone();
            new_permissions.capabilities.remove(&capability);

            let new_arc = Arc::new(new_permissions);
            policies.insert(process_id, new_arc);

            drop(policies);
            self.invalidate_cache_for_process(process_id).await;
            self.auditor
                .log_permission_revoke(process_id, &capability)
                .await;

            Ok(())
        } else {
            Err(PermissionError::ProcessNotFound)
        }
    }

    pub async fn check_permission(
        &self,
        process_id: u32,
        capability: &Capability,
    ) -> Result<bool, PermissionError> {
        self.metrics.total_checks.fetch_add(1, Ordering::Relaxed);

        if let Some(cached_result) = self.get_cached_permission(process_id, capability).await {
            self.metrics.cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(cached_result);
        }

        let (has_permission, last_accessed) = {
            let policies = self.policies.read().await;
            if let Some(permissions) = policies.get(&process_id) {
                let now = SystemTime::now();

                if let Some(expires_at) = permissions.expires_at {
                    if now > expires_at {
                        return Ok(false);
                    }
                }

                let has_permission = permissions.capabilities.contains(capability);
                let last_accessed = Arc::clone(&permissions.last_accessed);

                (has_permission, last_accessed)
            } else {
                return Err(PermissionError::ProcessNotFound);
            }
        };

        let now = SystemTime::now();
        last_accessed.store(
            now.duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            Ordering::Relaxed,
        );

        self.cache_permission_result(process_id, capability.clone(), has_permission)
            .await;
        self.auditor
            .log_permission_check(process_id, capability, has_permission)
            .await;

        Ok(has_permission)
    }

    pub async fn check_permission_batch(
        &self,
        process_id: u32,
        capabilities: &[Capability],
    ) -> Result<Vec<bool>, PermissionError> {
        let policies = self.policies.read().await;
        if let Some(permissions) = policies.get(&process_id) {
            let now = SystemTime::now();

            if let Some(expires_at) = permissions.expires_at {
                if now > expires_at {
                    return Ok(vec![false; capabilities.len()]);
                }
            }

            let results: Vec<bool> = capabilities
                .iter()
                .map(|cap| permissions.capabilities.contains(cap))
                .collect();

            permissions.last_accessed.store(
                now.duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                Ordering::Relaxed,
            );

            Ok(results)
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

    pub async fn check_resource_usage(
        &self,
        process_id: u32,
        usage: &ResourceUsage,
    ) -> Result<bool, PermissionError> {
        let (within_limits, limits_copy) = {
            let policies = self.policies.read().await;
            if let Some(permissions) = policies.get(&process_id) {
                let limits = &permissions.resource_limits;

                let violations = [
                    (usage.memory_bytes > limits.max_memory_bytes, "memory"),
                    (usage.cpu_percent > limits.max_cpu_percent, "cpu"),
                    (
                        usage.file_descriptors > limits.max_file_descriptors,
                        "file_descriptors",
                    ),
                    (
                        usage.network_connections > limits.max_network_connections,
                        "network_connections",
                    ),
                ];

                let within_limits = !violations.iter().any(|(violated, _)| *violated);

                (within_limits, limits.clone())
            } else {
                return Err(PermissionError::ProcessNotFound);
            }
        };

        if !within_limits {
            self.auditor
                .log_resource_violation(process_id, usage, &limits_copy)
                .await;
        }

        Ok(within_limits)
    }

    async fn cleanup_expired_permissions_background(
        policies: Arc<RwLock<HashMap<u32, Arc<ProcessPermissions>>>>,
        auditor: Arc<SecurityAuditor>,
        metrics: Arc<PermissionMetrics>,
    ) {
        let now = SystemTime::now();
        let mut expired_processes = Vec::new();

        {
            let mut policies_guard = policies.write().await;
            policies_guard.retain(|&process_id, permissions| {
                if let Some(expires_at) = permissions.expires_at {
                    if now > expires_at {
                        expired_processes.push(process_id);
                        return false;
                    }
                }
                true
            });
        }

        if !expired_processes.is_empty() {
            metrics
                .expired_cleanups
                .fetch_add(expired_processes.len() as u64, Ordering::Relaxed);

            for process_id in expired_processes {
                auditor.log_permission_expiry(process_id).await;
            }
        }
    }

    async fn cleanup_cache_background(
        cache: Arc<RwLock<HashMap<(u32, Capability), (bool, SystemTime)>>>,
    ) {
        let cutoff = SystemTime::now() - Duration::from_secs(600);
        let mut cache_guard = cache.write().await;
        cache_guard.retain(|_, (_, timestamp)| *timestamp > cutoff);
    }

    async fn get_cached_permission(
        &self,
        process_id: u32,
        capability: &Capability,
    ) -> Option<bool> {
        let cache = self.capability_cache.read().await;
        if let Some((result, timestamp)) = cache.get(&(process_id, capability.clone())) {
            if SystemTime::now()
                .duration_since(*timestamp)
                .unwrap_or_default()
                < Duration::from_secs(60)
            {
                return Some(*result);
            }
        }
        None
    }

    async fn cache_permission_result(&self, process_id: u32, capability: Capability, result: bool) {
        let mut cache = self.capability_cache.write().await;
        cache.insert((process_id, capability), (result, SystemTime::now()));

        if cache.len() > 10000 {
            let cutoff = SystemTime::now() - Duration::from_secs(300);
            cache.retain(|_, (_, timestamp)| *timestamp > cutoff);
        }
    }

    async fn invalidate_cache_for_process(&self, process_id: u32) {
        let mut cache = self.capability_cache.write().await;
        cache.retain(|(pid, _), _| *pid != process_id);
    }

    pub async fn get_process_permissions(&self, process_id: u32) -> Option<ProcessPermissions> {
        let policies = self.policies.read().await;
        policies.get(&process_id).map(|arc| (**arc).clone())
    }

    pub async fn update_resource_limits(
        &self,
        process_id: u32,
        new_limits: ResourceLimits,
    ) -> Result<(), PermissionError> {
        let mut policies = self.policies.write().await;

        if let Some(permissions_arc) = policies.get(&process_id) {
            let mut new_permissions = (**permissions_arc).clone();
            new_permissions.resource_limits = new_limits.clone();

            let new_arc = Arc::new(new_permissions);
            policies.insert(process_id, new_arc);

            drop(policies);
            self.auditor
                .log_resource_limit_update(process_id, &new_limits)
                .await;

            Ok(())
        } else {
            Err(PermissionError::ProcessNotFound)
        }
    }

    pub async fn get_security_report(&self) -> SecurityReport {
        self.auditor.generate_security_report().await
    }

    pub async fn get_metrics(&self) -> PermissionManagerMetrics {
        PermissionManagerMetrics {
            total_checks: self.metrics.total_checks.load(Ordering::Relaxed),
            cache_hits: self.metrics.cache_hits.load(Ordering::Relaxed),
            cache_hit_rate: {
                let total = self.metrics.total_checks.load(Ordering::Relaxed);
                if total > 0 {
                    (self.metrics.cache_hits.load(Ordering::Relaxed) as f64 / total as f64) * 100.0
                } else {
                    0.0
                }
            },
            permission_grants: self.metrics.permission_grants.load(Ordering::Relaxed),
            permission_denials: self.metrics.permission_denials.load(Ordering::Relaxed),
            expired_cleanups: self.metrics.expired_cleanups.load(Ordering::Relaxed),
            active_processes: self.policies.read().await.len() as u64,
        }
    }

    pub async fn shutdown(&mut self) -> Result<(), PermissionError> {
        self.shutdown_signal.notify_waiters();

        if let Some(handle) = self.cleanup_handle.take() {
            handle.abort();
        }

        self.auditor.shutdown().await;
        Ok(())
    }
}

impl CapabilityChecker {
    async fn new() -> Self {
        let mut system_capabilities = HashSet::new();
        system_capabilities.insert(Capability::ReadFiles("/tmp/*".to_string()));
        system_capabilities.insert(Capability::NetworkRequest("https://*".to_string()));
        system_capabilities.insert(Capability::ExecuteScript);
        system_capabilities.insert(Capability::ModifyDom);
        system_capabilities.insert(Capability::CacheAccess);
        system_capabilities.insert(Capability::StorageAccess);

        let platform_features = PlatformFeatures::detect().await;
        let capability_matrix =
            Self::build_capability_matrix(&system_capabilities, &platform_features);

        Self {
            system_capabilities,
            capability_matrix,
            platform_features,
        }
    }

    fn build_capability_matrix(
        capabilities: &HashSet<Capability>,
        features: &PlatformFeatures,
    ) -> HashMap<Capability, bool> {
        let mut matrix = HashMap::new();

        for capability in capabilities {
            matrix.insert(capability.clone(), true);
        }

        matrix.insert(Capability::AccessCamera, features.has_camera);
        matrix.insert(Capability::AccessMicrophone, features.has_microphone);
        matrix.insert(Capability::AccessGpu, features.has_gpu);
        matrix.insert(Capability::AccessSerialPort, features.has_serial_ports);
        matrix.insert(Capability::AccessUsb, features.has_usb);

        matrix
    }

    async fn validate_capabilities_batch(&self, capabilities: &HashSet<Capability>) -> bool {
        capabilities.iter().all(|cap| {
            self.capability_matrix.get(cap).copied().unwrap_or(false)
                || self.is_capability_available(cap)
        })
    }

    fn is_capability_available(&self, capability: &Capability) -> bool {
        match capability {
            Capability::AccessSerialPort => self.platform_features.has_serial_ports,
            Capability::AccessUsb => self.platform_features.has_usb,
            Capability::AccessCamera => self.platform_features.has_camera,
            Capability::AccessMicrophone => self.platform_features.has_microphone,
            Capability::AccessGpu => self.platform_features.has_gpu,
            _ => self.system_capabilities.contains(capability),
        }
    }
}

impl PlatformFeatures {
    async fn detect() -> Self {
        Self {
            has_camera: Self::detect_camera().await,
            has_microphone: Self::detect_microphone().await,
            has_gpu: Self::detect_gpu().await,
            has_serial_ports: Self::detect_serial_ports().await,
            has_usb: Self::detect_usb().await,
        }
    }

    async fn detect_camera() -> bool {
        cfg!(target_os = "linux") || cfg!(target_os = "windows") || cfg!(target_os = "macos")
    }

    async fn detect_microphone() -> bool {
        cfg!(target_os = "linux") || cfg!(target_os = "windows") || cfg!(target_os = "macos")
    }

    async fn detect_gpu() -> bool {
        true
    }

    async fn detect_serial_ports() -> bool {
        cfg!(target_os = "linux") || cfg!(target_os = "windows")
    }

    async fn detect_usb() -> bool {
        cfg!(target_os = "linux") || cfg!(target_os = "windows") || cfg!(target_os = "macos")
    }
}

impl PermissionMetrics {
    fn new() -> Self {
        Self {
            total_checks: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            permission_grants: AtomicU64::new(0),
            permission_denials: AtomicU64::new(0),
            expired_cleanups: AtomicU64::new(0),
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

#[derive(Debug, Clone, Serialize)]
pub struct PermissionManagerMetrics {
    pub total_checks: u64,
    pub cache_hits: u64,
    pub cache_hit_rate: f64,
    pub permission_grants: u64,
    pub permission_denials: u64,
    pub expired_cleanups: u64,
    pub active_processes: u64,
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
    #[error("Manager shutdown")]
    ManagerShutdown,
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
            allowed_message_types: ["RenderCommand", "DomUpdate"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
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
