pub mod ipc;
pub mod permissions;
pub mod process;
pub mod security;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::sandbox::security::policy::{
    EnforcementMode, PolicyAction, PolicyEvaluationResult,
    PolicyViolation as PolicyEngineViolation, PolicyViolationType,
    SecurityAuditReport as PolicyEngineAuditReport, SecurityPolicyEngine,
};
use crate::sandbox::security::{
    SecurityAnalysisResult, SecurityEvent, SecurityEventType, SecurityFramework, SecuritySeverity,
    SecurityStatus, ThreatLevel,
};

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
    pub security_status: SecurityStatus,
    pub policy_engine_report: PolicyEngineAuditReport,
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
    PolicyViolation,
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
    security_framework: Arc<SecurityFramework>,
    policy_engine: Arc<RwLock<SecurityPolicyEngine>>,
    max_processes: u32,
}

impl SandboxManager {
    pub async fn new() -> Result<Self, SandboxError> {
        Self::with_config(SandboxConfig::default()).await
    }

    pub async fn with_config(config: SandboxConfig) -> Result<Self, SandboxError> {
        let permission_manager = Arc::new(permissions::PermissionManager::new().await?);
        let ipc_manager = Arc::new(ipc::IpcManager::new());
        let security_framework = Arc::new(SecurityFramework::new());
        let policy_engine = Arc::new(RwLock::new(SecurityPolicyEngine::new()));

        Ok(Self {
            processes: Arc::new(RwLock::new(HashMap::with_capacity(config.initial_capacity))),
            permission_manager,
            security_policy: config.security_policy,
            ipc_manager,
            security_framework,
            policy_engine,
            max_processes: config.max_processes,
        })
    }

    pub async fn create_sandboxed_process(
        &self,
        config: process::ProcessConfig,
    ) -> Result<ProcessId, SandboxError> {
        self.validate_process_creation(&config).await?;

        let process_id = Self::generate_process_id();
        let creation_event = self.build_process_creation_event(process_id, &config);
        let (evaluation, analysis, blocking_reason) =
            self.evaluate_security_event(creation_event).await;

        if let Some(reason) = blocking_reason {
            error!(
                target: "sandbox::security",
                "Security policy denied process {} creation: {}",
                process_id,
                reason
            );
            return Err(SandboxError::SecurityViolation(reason));
        }

        if !evaluation.violations.is_empty() {
            warn!(
                target: "sandbox::security",
                "Process {} triggered policy warnings: {:?}",
                process_id,
                evaluation
                    .violations
                    .iter()
                    .map(|violation| violation.rule_id.as_str())
                    .collect::<Vec<_>>()
            );
        }

        if matches!(
            analysis.threat_level,
            ThreatLevel::Medium | ThreatLevel::High | ThreatLevel::Critical
        ) || analysis.threat_score > 0.5
        {
            info!(
                target: "sandbox::security",
                "Process {} security profile initialized with threat level {:?} (score {:.2})",
                process_id,
                analysis.threat_level,
                analysis.threat_score
            );
        }

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

        let ipc_event = self.build_ipc_event(from, to, &message);
        let (evaluation, analysis, blocking_reason) = self.evaluate_security_event(ipc_event).await;

        if let Some(reason) = blocking_reason {
            return Err(SandboxError::SecurityViolation(reason));
        }

        if matches!(
            analysis.threat_level,
            ThreatLevel::High | ThreatLevel::Critical
        ) {
            warn!(
                target: "sandbox::security",
                "IPC message with priority {:?} escalated threat level {:?} between {} and {}",
                message.priority,
                analysis.threat_level,
                from,
                to
            );
        }

        if !evaluation.violations.is_empty() {
            debug!(
                target: "sandbox::security",
                "IPC message {} triggered policy violations {:?}",
                message.id,
                evaluation
                    .violations
                    .iter()
                    .map(|violation| violation.rule_id.as_str())
                    .collect::<Vec<_>>()
            );
        }

        self.ipc_manager
            .send_message(from, to, message)
            .await
            .map_err(SandboxError::IpcError)
    }

    pub async fn audit_security(&self) -> Result<SecurityAuditReport, SandboxError> {
        let mut violations = Vec::new();
        let mut total_memory = 0u64;
        let mut total_cpu_usage = 0f64;
        let mut total_threads = 0u32;
        let mut total_file_handles = 0u32;
        let total_processes;

        {
            let processes = self.processes.read().await;
            total_processes = processes.len();

            for (process_id, process) in processes.iter() {
                let stats = process.get_stats().await;
                total_memory += stats.memory_usage_bytes;
                total_cpu_usage += stats.cpu_usage_percent;
                total_threads += stats.thread_count;
                total_file_handles += stats.file_handles_count;

                let process_violations = self.check_process_violations(*process_id, &stats).await;
                violations.extend(process_violations);
            }
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

        let security_status = self.security_framework.get_security_status().await;
        let policy_engine_report = {
            let engine = self.policy_engine.read().await;
            engine.create_audit_report().await
        };

        Ok(SecurityAuditReport {
            timestamp: Self::current_timestamp(),
            total_processes,
            security_violations: violations,
            resource_usage: ResourceUsageReport {
                total_memory_used: total_memory,
                total_cpu_usage,
                network_connections: 0,
                file_descriptors: total_file_handles,
                total_threads,
            },
            compliance_status,
            security_status,
            policy_engine_report,
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

    async fn evaluate_security_event(
        &self,
        event: SecurityEvent,
    ) -> (
        PolicyEvaluationResult,
        SecurityAnalysisResult,
        Option<String>,
    ) {
        let (evaluation, enforcement_mode) = {
            let engine_guard = self.policy_engine.read().await;
            let mode = engine_guard.get_enforcement_mode();
            let evaluation = engine_guard.evaluate_event(&event).await;
            (evaluation, mode)
        };

        let analysis = self
            .security_framework
            .analyze_security_event(event.clone())
            .await;

        if !analysis.recommended_actions.is_empty() {
            let recommended: Vec<String> = analysis
                .recommended_actions
                .iter()
                .map(|action| format!("{:?}", action))
                .collect();
            debug!(
                target: "sandbox::security",
                "Recommended actions for process {}: {} (threat {:.2})",
                event.source_process,
                recommended.join(", "),
                analysis.threat_score
            );
        }

        if !analysis.compliance_impact.required_actions.is_empty() {
            debug!(
                target: "sandbox::security",
                "Compliance actions required for process {}: {:?}",
                event.source_process,
                analysis.compliance_impact.required_actions
            );
        }

        if !analysis.compliance_impact.affected_frameworks.is_empty() {
            debug!(
                target: "sandbox::security",
                "Compliance frameworks impacted for process {}: {:?} (risk {:?})",
                event.source_process,
                analysis.compliance_impact.affected_frameworks,
                analysis.compliance_impact.risk_level
            );
        }

        let blocking_action = evaluation
            .applied_actions
            .iter()
            .find(|action| {
                matches!(
                    action.action,
                    PolicyAction::Deny
                        | PolicyAction::Block
                        | PolicyAction::Terminate
                        | PolicyAction::Quarantine
                )
            })
            .map(|action| action.action.clone());

        let mut reasons: Vec<String> = evaluation
            .violations
            .iter()
            .map(|violation| {
                format!(
                    "{} (policy {} rule {})",
                    violation.description, violation.policy_id, violation.rule_id
                )
            })
            .collect();

        if let Some(action) = &blocking_action {
            reasons.push(format!("Enforcement action {:?} triggered", action));
        }

        if matches!(
            analysis.threat_level,
            ThreatLevel::High | ThreatLevel::Critical
        ) {
            reasons.push(format!(
                "Threat level {:?} detected (score {:.2})",
                analysis.threat_level, analysis.threat_score
            ));
        }

        let message = match enforcement_mode {
            EnforcementMode::Enforcing if !reasons.is_empty() => Some(reasons.join("; ")),
            EnforcementMode::Complaining if blocking_action.is_some() => {
                if let Some(action) = blocking_action.as_ref() {
                    warn!(
                        target: "sandbox::security",
                        "Security policy would enforce {:?} for process {}: {}",
                        action,
                        event.source_process,
                        reasons.join("; ")
                    );
                }
                None
            }
            _ => None,
        };

        (evaluation, analysis, message)
    }

    fn map_policy_violation(
        process_id: ProcessId,
        violation: &PolicyEngineViolation,
        timestamp: u64,
    ) -> SecurityViolation {
        SecurityViolation {
            process_id,
            violation_type: Self::map_policy_violation_type(&violation.violation_type),
            severity: Self::map_policy_severity(violation.severity),
            description: violation.description.clone(),
            timestamp,
        }
    }

    fn map_policy_violation_type(violation_type: &PolicyViolationType) -> ViolationType {
        match violation_type {
            PolicyViolationType::UnauthorizedFileAccess => ViolationType::FileSystemViolation,
            PolicyViolationType::UnauthorizedNetworkAccess => {
                ViolationType::UnauthorizedNetworkAccess
            }
            PolicyViolationType::PrivilegeEscalation => ViolationType::UnauthorizedSyscall,
            PolicyViolationType::ResourceLimitExceeded => ViolationType::ExcessiveMemoryUsage,
            PolicyViolationType::SuspiciousSystemCall => ViolationType::UnauthorizedSyscall,
            PolicyViolationType::MaliciousActivity => ViolationType::PolicyViolation,
        }
    }

    fn map_policy_severity(severity: SecuritySeverity) -> Severity {
        match severity {
            SecuritySeverity::Critical => Severity::Critical,
            SecuritySeverity::High => Severity::High,
            SecuritySeverity::Medium => Severity::Medium,
            SecuritySeverity::Low => Severity::Low,
            SecuritySeverity::Info => Severity::Low,
        }
    }

    fn compute_threat_score(&self, stats: &process::ProcessStats) -> f64 {
        let memory_component = if self.security_policy.max_memory_per_process == 0 {
            0.0
        } else {
            (stats.memory_usage_bytes as f64 / self.security_policy.max_memory_per_process as f64)
                .min(1.5)
        };

        let cpu_component = (stats.cpu_usage_percent / 100.0).min(1.5);
        let fd_component = (stats.file_handles_count as f64 / 512.0).min(1.5);
        let thread_component = if stats.thread_count == 0 {
            0.0
        } else {
            (stats.thread_count as f64 / 128.0).min(1.5)
        };

        let components = [
            memory_component,
            cpu_component,
            fd_component,
            thread_component,
        ];
        let (sum, count) = components
            .iter()
            .fold((0.0, 0usize), |(sum, count), value| {
                if *value > 0.0 {
                    (sum + value, count + 1)
                } else {
                    (sum, count)
                }
            });

        if count == 0 {
            0.0
        } else {
            (sum / count as f64).clamp(0.0, 1.0)
        }
    }

    fn severity_from_score(score: f64) -> SecuritySeverity {
        if score >= 0.9 {
            SecuritySeverity::Critical
        } else if score >= 0.7 {
            SecuritySeverity::High
        } else if score >= 0.4 {
            SecuritySeverity::Medium
        } else if score >= 0.2 {
            SecuritySeverity::Low
        } else {
            SecuritySeverity::Info
        }
    }

    fn build_stats_event(
        &self,
        process_id: ProcessId,
        stats: &process::ProcessStats,
    ) -> SecurityEvent {
        let score = self.compute_threat_score(stats);
        let mut details = HashMap::new();
        details.insert("resource_type".to_string(), "process_stats".to_string());
        details.insert(
            "cpu_usage_percent".to_string(),
            format!("{:.2}", stats.cpu_usage_percent),
        );
        details.insert(
            "memory_usage_bytes".to_string(),
            stats.memory_usage_bytes.to_string(),
        );
        details.insert(
            "file_handles_count".to_string(),
            stats.file_handles_count.to_string(),
        );
        details.insert("thread_count".to_string(), stats.thread_count.to_string());
        details.insert(
            "network_bytes_sent".to_string(),
            stats.network_bytes_sent.to_string(),
        );
        details.insert(
            "network_bytes_received".to_string(),
            stats.network_bytes_received.to_string(),
        );
        if let Some(pid) = stats.pid {
            details.insert("pid".to_string(), pid.to_string());
        }

        SecurityEvent {
            timestamp: Self::current_timestamp(),
            event_type: SecurityEventType::ResourceAbuse,
            severity: Self::severity_from_score(score),
            source_process: process_id,
            target_resource: format!("process://{process_id}"),
            details,
            threat_score: score,
        }
    }

    fn build_process_creation_event(
        &self,
        process_id: ProcessId,
        config: &process::ProcessConfig,
    ) -> SecurityEvent {
        let mut details = HashMap::new();
        details.insert("executable".to_string(), config.executable_path.clone());
        details.insert("arg_count".to_string(), config.arguments.len().to_string());
        details.insert(
            "isolation_level".to_string(),
            format!("{:?}", config.isolation_level),
        );
        details.insert(
            "max_memory_mb".to_string(),
            config.resource_limits.max_memory_mb.to_string(),
        );
        details.insert(
            "max_cpu_percent".to_string(),
            config.resource_limits.max_cpu_percent.to_string(),
        );
        details.insert(
            "allow_network".to_string(),
            config.network_restrictions.allow_network.to_string(),
        );
        if !config.network_restrictions.allowed_domains.is_empty() {
            details.insert(
                "allowed_domains".to_string(),
                config.network_restrictions.allowed_domains.join(","),
            );
        }
        if !config.file_system_restrictions.read_only_paths.is_empty() {
            details.insert(
                "read_only_paths".to_string(),
                config.file_system_restrictions.read_only_paths.join(","),
            );
        }

        let requested_bytes = config
            .resource_limits
            .max_memory_mb
            .saturating_mul(1024 * 1024);
        let score = if self.security_policy.max_memory_per_process == 0 {
            0.0
        } else {
            (requested_bytes as f64 / self.security_policy.max_memory_per_process as f64)
                .clamp(0.0, 1.0)
        };

        SecurityEvent {
            timestamp: Self::current_timestamp(),
            event_type: SecurityEventType::AnomalousActivity,
            severity: Self::severity_from_score(score.max(0.2)),
            source_process: process_id,
            target_resource: config.executable_path.clone(),
            details,
            threat_score: score,
        }
    }

    fn build_ipc_event(
        &self,
        from: ProcessId,
        to: ProcessId,
        message: &ipc::IpcMessage,
    ) -> SecurityEvent {
        let mut details = HashMap::new();
        details.insert("message_id".to_string(), message.id.to_string());
        details.insert("priority".to_string(), format!("{:?}", message.priority));
        details.insert(
            "message_type".to_string(),
            match &message.message_type {
                ipc::MessageType::Custom(kind) => kind.clone(),
                other => format!("{:?}", other),
            },
        );
        details.insert(
            "payload_size".to_string(),
            message.payload.len().to_string(),
        );

        let score = match message.priority {
            ipc::MessagePriority::Critical => 0.9,
            ipc::MessagePriority::High => 0.7,
            ipc::MessagePriority::Normal => 0.4,
            ipc::MessagePriority::Low => 0.1,
        };

        SecurityEvent {
            timestamp: Self::current_timestamp(),
            event_type: SecurityEventType::PolicyViolation,
            severity: Self::severity_from_score(score),
            source_process: from,
            target_resource: format!("process://{to}"),
            details,
            threat_score: score,
        }
    }

    fn current_timestamp() -> u64 {
        use std::time::{Duration, SystemTime, UNIX_EPOCH};

        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs()
    }

    async fn check_process_violations(
        &self,
        process_id: ProcessId,
        stats: &process::ProcessStats,
    ) -> Vec<SecurityViolation> {
        let mut violations = Vec::new();
        let timestamp = Self::current_timestamp();

        let stats_event = self.build_stats_event(process_id, stats);
        let (evaluation, analysis, blocking_reason) =
            self.evaluate_security_event(stats_event).await;

        if let Some(reason) = blocking_reason {
            violations.push(SecurityViolation {
                process_id,
                violation_type: ViolationType::PolicyViolation,
                severity: Severity::Critical,
                description: reason,
                timestamp,
            });
        }

        for violation in &evaluation.violations {
            violations.push(Self::map_policy_violation(process_id, violation, timestamp));
        }

        if matches!(
            analysis.threat_level,
            ThreatLevel::High | ThreatLevel::Critical
        ) {
            violations.push(SecurityViolation {
                process_id,
                violation_type: ViolationType::UnauthorizedSyscall,
                severity: if analysis.threat_level == ThreatLevel::Critical {
                    Severity::Critical
                } else {
                    Severity::High
                },
                description: format!(
                    "Elevated threat level {:?} detected (score {:.2})",
                    analysis.threat_level, analysis.threat_score
                ),
                timestamp,
            });
        }

        if !analysis.recommended_actions.is_empty() {
            let actions = analysis
                .recommended_actions
                .iter()
                .map(|action| format!("{:?}", action))
                .collect::<Vec<_>>()
                .join(", ");
            violations.push(SecurityViolation {
                process_id,
                violation_type: ViolationType::PolicyViolation,
                severity: Severity::Medium,
                description: format!("Recommended remediation actions: {actions}"),
                timestamp,
            });
        }

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
