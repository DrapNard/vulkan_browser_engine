use super::{Capability, ProcessPermissions, ResourceLimits, ResourceUsage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use tracing::log;

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