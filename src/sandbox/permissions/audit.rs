use super::{Capability, ProcessPermissions, ResourceLimits, ResourceUsage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicU32, Ordering};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::{RwLock, mpsc, Notify};
use tracing::log;

pub struct SecurityAuditor {
    event_buffer: Arc<RwLock<CircularBuffer<AuditEvent>>>,
    file_writer: Arc<FileWriter>,
    metrics: Arc<AtomicMetrics>,
    event_sender: mpsc::UnboundedSender<AuditEvent>,
    shutdown_signal: Arc<Notify>,
}

struct CircularBuffer<T> {
    buffer: Vec<Option<T>>,
    head: usize,
    size: usize,
    capacity: usize,
}

struct FileWriter {
    sender: mpsc::UnboundedSender<String>,
    file_handle: Arc<tokio::sync::Mutex<Option<tokio::fs::File>>>,
}

struct AtomicMetrics {
    total_events: AtomicU64,
    security_violations: AtomicU64,
    permission_denials: AtomicU64,
    resource_violations: AtomicU64,
    active_processes: AtomicU32,
    high_risk_processes: AtomicU32,
    severity_counters: [AtomicU64; 5],
    process_counters: Arc<RwLock<HashMap<u32, AtomicU64>>>,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SeverityLevel {
    Critical = 0,
    High = 1,
    Medium = 2,
    Low = 3,
    Info = 4,
}

#[derive(Debug, Default, Clone, Serialize)]
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

#[derive(Debug, Clone, Copy, Serialize, PartialEq, PartialOrd)]
pub enum RiskLevel {
    Low = 0,
    Medium = 1,
    High = 2,
    Critical = 3,
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

pub struct CircularBufferIterator<'a, T> {
    buffer: &'a [Option<T>],
    start: usize,
    end: usize,
    len: usize,
    capacity: usize,
}

impl<T> CircularBuffer<T> {
    fn new(capacity: usize) -> Self {
        Self {
            buffer: (0..capacity).map(|_| None).collect(),
            head: 0,
            size: 0,
            capacity,
        }
    }

    fn push(&mut self, item: T) {
        self.buffer[self.head] = Some(item);
        self.head = (self.head + 1) % self.capacity;
        if self.size < self.capacity {
            self.size += 1;
        }
    }

    fn iter(&self) -> CircularBufferIterator<T> {
        let start = if self.size == self.capacity {
            self.head
        } else {
            0
        };
        
        CircularBufferIterator {
            buffer: &self.buffer,
            start,
            end: (start + self.size) % self.capacity,
            len: self.size,
            capacity: self.capacity,
        }
    }

    fn len(&self) -> usize {
        self.size
    }
}

impl<'a, T> Iterator for CircularBufferIterator<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.len == 0 {
            return None;
        }

        let current = self.start;
        self.start = (self.start + 1) % self.capacity;
        self.len -= 1;

        self.buffer[current].as_ref()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len, Some(self.len))
    }
}

impl<'a, T> DoubleEndedIterator for CircularBufferIterator<'a, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.len == 0 {
            return None;
        }

        self.end = (self.end + self.capacity - 1) % self.capacity;
        self.len -= 1;

        self.buffer[self.end].as_ref()
    }
}

impl<'a, T> ExactSizeIterator for CircularBufferIterator<'a, T> {
    fn len(&self) -> usize {
        self.len
    }
}

impl AtomicMetrics {
    fn new() -> Self {
        Self {
            total_events: AtomicU64::new(0),
            security_violations: AtomicU64::new(0),
            permission_denials: AtomicU64::new(0),
            resource_violations: AtomicU64::new(0),
            active_processes: AtomicU32::new(0),
            high_risk_processes: AtomicU32::new(0),
            severity_counters: [
                AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0),
                AtomicU64::new(0), AtomicU64::new(0),
            ],
            process_counters: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn increment_event(&self, event: &AuditEvent) {
        self.total_events.fetch_add(1, Ordering::Relaxed);
        self.severity_counters[event.severity as usize].fetch_add(1, Ordering::Relaxed);

        match event.event_type {
            AuditEventType::SecurityViolation => {
                self.security_violations.fetch_add(1, Ordering::Relaxed);
            }
            AuditEventType::PermissionDenied => {
                self.permission_denials.fetch_add(1, Ordering::Relaxed);
            }
            AuditEventType::ResourceViolation => {
                self.resource_violations.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }

        let mut process_counters = self.process_counters.write().await;
        process_counters
            .entry(event.process_id)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    async fn to_security_metrics(&self) -> SecurityMetrics {
        let process_counters = self.process_counters.read().await;
        
        SecurityMetrics {
            total_events: self.total_events.load(Ordering::Relaxed),
            security_violations: self.security_violations.load(Ordering::Relaxed),
            permission_denials: self.permission_denials.load(Ordering::Relaxed),
            resource_violations: self.resource_violations.load(Ordering::Relaxed),
            active_processes: self.active_processes.load(Ordering::Relaxed),
            high_risk_processes: self.high_risk_processes.load(Ordering::Relaxed),
            events_by_severity: [
                (SeverityLevel::Critical, self.severity_counters[0].load(Ordering::Relaxed)),
                (SeverityLevel::High, self.severity_counters[1].load(Ordering::Relaxed)),
                (SeverityLevel::Medium, self.severity_counters[2].load(Ordering::Relaxed)),
                (SeverityLevel::Low, self.severity_counters[3].load(Ordering::Relaxed)),
                (SeverityLevel::Info, self.severity_counters[4].load(Ordering::Relaxed)),
            ].into_iter().collect(),
            events_by_process: process_counters
                .iter()
                .map(|(pid, counter)| (*pid, counter.load(Ordering::Relaxed)))
                .collect(),
        }
    }
}

impl FileWriter {
    fn new() -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel::<String>();
        let file_handle = Arc::new(tokio::sync::Mutex::new(None));
        let file_handle_clone = Arc::clone(&file_handle);

        tokio::spawn(async move {
            let mut batch = Vec::with_capacity(100);
            let mut flush_interval = tokio::time::interval(std::time::Duration::from_millis(500));

            loop {
                tokio::select! {
                    msg = receiver.recv() => {
                        match msg {
                            Some(line) => {
                                batch.push(line);
                                if batch.len() >= 100 {
                                    Self::flush_batch(&file_handle_clone, &mut batch).await;
                                }
                            }
                            None => break,
                        }
                    }
                    _ = flush_interval.tick() => {
                        if !batch.is_empty() {
                            Self::flush_batch(&file_handle_clone, &mut batch).await;
                        }
                    }
                }
            }

            if !batch.is_empty() {
                Self::flush_batch(&file_handle_clone, &mut batch).await;
            }
        });

        Self { sender, file_handle }
    }

    async fn flush_batch(file_handle: &Arc<tokio::sync::Mutex<Option<tokio::fs::File>>>, batch: &mut Vec<String>) {
        if let Some(file) = file_handle.lock().await.as_mut() {
            let combined = batch.join("\n") + "\n";
            if let Err(e) = file.write_all(combined.as_bytes()).await {
                log::error!("Failed to write audit batch to file: {}", e);
            } else if let Err(e) = file.flush().await {
                log::error!("Failed to flush audit file: {}", e);
            }
        }
        batch.clear();
    }

    async fn write_line(&self, line: String) {
        if self.sender.send(line).is_err() {
            log::error!("Audit file writer channel closed");
        }
    }

    async fn initialize(&self) {
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open("security_audit.log")
            .await
        {
            Ok(file) => {
                *self.file_handle.lock().await = Some(file);
                log::info!("Security audit log file initialized");
            }
            Err(e) => {
                log::error!("Failed to open audit log file: {}", e);
            }
        }
    }
}

impl SecurityAuditor {
    pub async fn new() -> Self {
        let (event_sender, mut event_receiver) = mpsc::unbounded_channel::<AuditEvent>();
        let event_buffer = Arc::new(RwLock::new(CircularBuffer::new(10000)));
        let file_writer = Arc::new(FileWriter::new());
        let metrics = Arc::new(AtomicMetrics::new());
        let shutdown_signal = Arc::new(Notify::new());

        file_writer.initialize().await;

        let buffer_clone = Arc::clone(&event_buffer);
        let file_writer_clone = Arc::clone(&file_writer);
        let metrics_clone = Arc::clone(&metrics);
        let shutdown_clone = Arc::clone(&shutdown_signal);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    event = event_receiver.recv() => {
                        match event {
                            Some(audit_event) => {
                                buffer_clone.write().await.push(audit_event.clone());
                                metrics_clone.increment_event(&audit_event).await;
                                
                                let json_line = serde_json::to_string(&audit_event)
                                    .unwrap_or_else(|_| "SERIALIZATION_ERROR".to_string());
                                file_writer_clone.write_line(json_line).await;
                            }
                            None => break,
                        }
                    }
                    _ = shutdown_clone.notified() => {
                        log::info!("SecurityAuditor event processor shutting down");
                        break;
                    }
                }
            }
        });

        Self {
            event_buffer,
            file_writer,
            metrics,
            event_sender,
            shutdown_signal,
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
        if self.event_sender.send(event).is_err() {
            log::error!("Failed to send audit event - channel closed");
        }
    }

    pub async fn generate_security_report(&self) -> SecurityReport {
        let metrics = self.metrics.to_security_metrics().await;
        let recent_violations = self.get_recent_violations().await;
        let risk_assessment = self.assess_risk(&metrics).await;
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
        let buffer = self.event_buffer.read().await;
        let cutoff_time = self.current_timestamp() - (24 * 60 * 60 * 1000);
        
        buffer.iter()
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

    async fn assess_risk(&self, metrics: &SecurityMetrics) -> RiskAssessment {
        let mut process_risk_scores = HashMap::new();
        let mut threat_indicators = Vec::new();
        
        for (process_id, event_count) in &metrics.events_by_process {
            if *event_count == 0 { continue; }
            
            let violation_count = self.count_process_violations(*process_id).await;
            let risk_score = (violation_count as f64 / *event_count as f64) * 100.0;
            process_risk_scores.insert(*process_id, risk_score);
            
            if risk_score > 20.0 {
                threat_indicators.push(ThreatIndicator {
                    indicator_type: "High violation rate".to_string(),
                    description: format!("Process {} has {:.1}% violation rate", process_id, risk_score),
                    confidence: (risk_score / 100.0).min(1.0),
                    first_seen: self.current_timestamp() - 86400000,
                    last_seen: self.current_timestamp(),
                    count: violation_count,
                });
            }
        }

        let overall_risk_level = match () {
            _ if metrics.security_violations > 10 => RiskLevel::Critical,
            _ if metrics.security_violations > 5 => RiskLevel::High,
            _ if metrics.resource_violations > 20 => RiskLevel::Medium,
            _ => RiskLevel::Low,
        };

        RiskAssessment {
            overall_risk_level,
            process_risk_scores,
            threat_indicators,
        }
    }

    async fn count_process_violations(&self, process_id: u32) -> u32 {
        let buffer = self.event_buffer.read().await;
        buffer.iter()
            .filter(|e| e.process_id == process_id && 
                matches!(e.event_type, AuditEventType::SecurityViolation | AuditEventType::ResourceViolation))
            .count() as u32
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
        let buffer = self.event_buffer.read().await;
        let filtered: Vec<AuditEvent> = buffer.iter()
            .filter(|event| process_id.map_or(true, |pid| event.process_id == pid))
            .rev()
            .take(limit.unwrap_or(100))
            .cloned()
            .collect();
        
        filtered
    }

    pub async fn get_metrics(&self) -> SecurityMetrics {
        self.metrics.to_security_metrics().await
    }

    pub async fn shutdown(&self) {
        self.shutdown_signal.notify_waiters();
        log::info!("SecurityAuditor shutdown signal sent");
    }
}

impl Default for SecurityAuditor {
    fn default() -> Self {
        futures::executor::block_on(Self::new())
    }
}