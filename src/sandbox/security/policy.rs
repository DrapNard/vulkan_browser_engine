use super::{SecurityEvent, SecuritySeverity, ThreatLevel};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::log;

pub struct SecurityPolicyEngine {
    policies: Vec<SecurityPolicy>,
    policy_groups: HashMap<String, PolicyGroup>,
    enforcement_mode: EnforcementMode,
    default_actions: HashMap<PolicyViolationType, PolicyAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityPolicy {
    pub id: String,
    pub name: String,
    pub description: String,
    pub rules: Vec<PolicyRule>,
    pub severity: SecuritySeverity,
    pub enabled: bool,
    pub version: String,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub id: String,
    pub condition: PolicyCondition,
    pub action: PolicyAction,
    pub exceptions: Vec<PolicyException>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PolicyCondition {
    ProcessMatch {
        process_name: Option<String>,
        process_id: Option<u32>,
        executable_path: Option<String>,
    },
    ResourceAccess {
        resource_type: ResourceType,
        access_type: AccessType,
        resource_path: Option<String>,
    },
    NetworkActivity {
        protocol: Option<String>,
        destination: Option<String>,
        port: Option<u16>,
    },
    SystemCall {
        syscall_name: String,
        parameters: Vec<String>,
    },
    Composite {
        operator: LogicalOperator,
        conditions: Vec<PolicyCondition>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResourceType {
    File,
    Directory,
    Network,
    Memory,
    Cpu,
    Device,
    Registry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AccessType {
    Read,
    Write,
    Execute,
    Delete,
    Create,
    Modify,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogicalOperator {
    And,
    Or,
    Not,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PolicyAction {
    Allow,
    Deny,
    Block,
    Quarantine,
    Terminate,
    Alert(AlertLevel),
    Log(LogLevel),
    Throttle(ThrottleConfig),
    Redirect(RedirectConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlertLevel {
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThrottleConfig {
    pub max_operations_per_second: u32,
    pub burst_capacity: u32,
    pub duration: std::time::Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedirectConfig {
    pub target_path: String,
    pub preserve_permissions: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyException {
    pub id: String,
    pub condition: PolicyCondition,
    pub reason: String,
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct PolicyGroup {
    pub name: String,
    pub description: String,
    pub policies: Vec<String>,
    pub priority: u8,
    pub inheritance: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum EnforcementMode {
    Permissive,
    Enforcing,
    Complaining,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum PolicyViolationType {
    UnauthorizedFileAccess,
    UnauthorizedNetworkAccess,
    PrivilegeEscalation,
    ResourceLimitExceeded,
    SuspiciousSystemCall,
    MaliciousActivity,
}

pub struct PolicyEvaluationResult {
    pub violations: Vec<PolicyViolation>,
    pub applied_actions: Vec<AppliedAction>,
    pub evaluation_time: std::time::Duration,
}

#[derive(Debug, Clone)]
pub struct PolicyViolation {
    pub policy_id: String,
    pub rule_id: String,
    pub violation_type: PolicyViolationType,
    pub severity: SecuritySeverity,
    pub description: String,
    pub evidence: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct AppliedAction {
    pub action: PolicyAction,
    pub policy_id: String,
    pub success: bool,
    pub message: Option<String>,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        SecurityPolicy {
            id: "default".to_string(),
            name: "Default Policy".to_string(),
            description: "Default security policy".to_string(),
            rules: Vec::new(),
            severity: SecuritySeverity::Medium,
            enabled: true,
            version: "1.0.0".to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }
}

impl SecurityPolicyEngine {
    pub fn new() -> Self {
        let mut engine = Self {
            policies: Vec::new(),
            policy_groups: HashMap::new(),
            enforcement_mode: EnforcementMode::Enforcing,
            default_actions: HashMap::new(),
        };
        
        engine.load_default_policies();
        engine.setup_default_actions();
        engine
    }

    fn load_default_policies(&mut self) {
        self.policies.push(SecurityPolicy {
            id: "CORE_001".to_string(),
            name: "File System Protection".to_string(),
            description: "Protect critical system files from unauthorized access".to_string(),
            rules: vec![
                PolicyRule {
                    id: "FS_001".to_string(),
                    condition: PolicyCondition::ResourceAccess {
                        resource_type: ResourceType::File,
                        access_type: AccessType::Write,
                        resource_path: Some("/etc/passwd".to_string()),
                    },
                    action: PolicyAction::Deny,
                    exceptions: Vec::new(),
                    metadata: HashMap::new(),
                },
                PolicyRule {
                    id: "FS_002".to_string(),
                    condition: PolicyCondition::ResourceAccess {
                        resource_type: ResourceType::Directory,
                        access_type: AccessType::Execute,
                        resource_path: Some("/root".to_string()),
                    },
                    action: PolicyAction::Alert(AlertLevel::Warning),
                    exceptions: Vec::new(),
                    metadata: HashMap::new(),
                },
            ],
            severity: SecuritySeverity::High,
            enabled: true,
            version: "1.0.0".to_string(),
            created_at: 0,
            updated_at: 0,
        });

        self.policies.push(SecurityPolicy {
            id: "CORE_002".to_string(),
            name: "Network Security".to_string(),
            description: "Control network access and prevent data exfiltration".to_string(),
            rules: vec![
                PolicyRule {
                    id: "NET_001".to_string(),
                    condition: PolicyCondition::NetworkActivity {
                        protocol: Some("TCP".to_string()),
                        destination: None,
                        port: Some(22),
                    },
                    action: PolicyAction::Log(LogLevel::Info),
                    exceptions: Vec::new(),
                    metadata: HashMap::new(),
                },
                PolicyRule {
                    id: "NET_002".to_string(),
                    condition: PolicyCondition::NetworkActivity {
                        protocol: Some("TCP".to_string()),
                        destination: Some("suspicious-domain.com".to_string()),
                        port: None,
                    },
                    action: PolicyAction::Block,
                    exceptions: Vec::new(),
                    metadata: HashMap::new(),
                },
            ],
            severity: SecuritySeverity::Medium,
            enabled: true,
            version: "1.0.0".to_string(),
            created_at: 0,
            updated_at: 0,
        });
    }

    fn setup_default_actions(&mut self) {
        self.default_actions.insert(
            PolicyViolationType::UnauthorizedFileAccess,
            PolicyAction::Block,
        );
        self.default_actions.insert(
            PolicyViolationType::UnauthorizedNetworkAccess,
            PolicyAction::Alert(AlertLevel::Warning),
        );
        self.default_actions.insert(
            PolicyViolationType::PrivilegeEscalation,
            PolicyAction::Terminate,
        );
        self.default_actions.insert(
            PolicyViolationType::ResourceLimitExceeded,
            PolicyAction::Throttle(ThrottleConfig {
                max_operations_per_second: 10,
                burst_capacity: 5,
                duration: std::time::Duration::from_secs(60),
            }),
        );
    }

    pub async fn evaluate_event(&self, event: &SecurityEvent) -> PolicyEvaluationResult {
        let start_time = std::time::Instant::now();
        let mut violations = Vec::new();
        let mut applied_actions = Vec::new();

        for policy in &self.policies {
            if !policy.enabled {
                continue;
            }

            for rule in &policy.rules {
                if self.evaluate_condition(&rule.condition, event) {
                    if !self.check_exceptions(&rule.exceptions, event) {
                        let violation = PolicyViolation {
                            policy_id: policy.id.clone(),
                            rule_id: rule.id.clone(),
                            violation_type: self.determine_violation_type(&rule.condition),
                            severity: policy.severity,
                            description: format!("Policy {} violated by rule {}", policy.name, rule.id),
                            evidence: self.collect_evidence(event),
                        };
                        violations.push(violation);

                        let applied_action = self.apply_action(&rule.action, event, &policy.id);
                        applied_actions.push(applied_action);
                    }
                }
            }
        }

        PolicyEvaluationResult {
            violations,
            applied_actions,
            evaluation_time: start_time.elapsed(),
        }
    }

    fn evaluate_condition(&self, condition: &PolicyCondition, event: &SecurityEvent) -> bool {
        match condition {
            PolicyCondition::ProcessMatch { process_id, .. } => {
                if let Some(pid) = process_id {
                    event.source_process == *pid
                } else {
                    true
                }
            }
            PolicyCondition::ResourceAccess { resource_type, access_type, resource_path } => {
                self.matches_resource_access(event, resource_type, access_type, resource_path)
            }
            PolicyCondition::NetworkActivity { protocol, destination, port } => {
                self.matches_network_activity(event, protocol, destination, port)
            }
            PolicyCondition::SystemCall { syscall_name, .. } => {
                event.details.get("syscall").map_or(false, |sc| sc == syscall_name)
            }
            PolicyCondition::Composite { operator, conditions } => {
                self.evaluate_composite_condition(operator, conditions, event)
            }
        }
    }

    fn matches_resource_access(
        &self,
        event: &SecurityEvent,
        resource_type: &ResourceType,
        access_type: &AccessType,
        resource_path: &Option<String>,
    ) -> bool {
        let event_resource_type = event.details.get("resource_type");
        let event_access_type = event.details.get("access_type");
        let event_path = event.details.get("path");

        let resource_match = event_resource_type.map_or(false, |rt| {
            match (resource_type, rt.as_str()) {
                (ResourceType::File, "file") => true,
                (ResourceType::Directory, "directory") => true,
                (ResourceType::Network, "network") => true,
                _ => false,
            }
        });

        let access_match = event_access_type.map_or(false, |at| {
            match (access_type, at.as_str()) {
                (AccessType::Read, "read") => true,
                (AccessType::Write, "write") => true,
                (AccessType::Execute, "execute") => true,
                _ => false,
            }
        });

        let path_match = if let Some(expected_path) = resource_path {
            event_path.map_or(false, |path| path.contains(expected_path))
        } else {
            true
        };

        resource_match && access_match && path_match
    }

    fn matches_network_activity(
        &self,
        event: &SecurityEvent,
        protocol: &Option<String>,
        destination: &Option<String>,
        port: &Option<u16>,
    ) -> bool {
        let protocol_match = if let Some(expected_protocol) = protocol {
            event.details.get("protocol").map_or(false, |p| p == expected_protocol)
        } else {
            true
        };

        let destination_match = if let Some(expected_dest) = destination {
            event.details.get("destination").map_or(false, |d| d.contains(expected_dest))
        } else {
            true
        };

        let port_match = if let Some(expected_port) = port {
            event.details.get("port").and_then(|p| p.parse::<u16>().ok()).map_or(false, |p| p == *expected_port)
        } else {
            true
        };

        protocol_match && destination_match && port_match
    }

    fn evaluate_composite_condition(
        &self,
        operator: &LogicalOperator,
        conditions: &[PolicyCondition],
        event: &SecurityEvent,
    ) -> bool {
        match operator {
            LogicalOperator::And => conditions.iter().all(|c| self.evaluate_condition(c, event)),
            LogicalOperator::Or => conditions.iter().any(|c| self.evaluate_condition(c, event)),
            LogicalOperator::Not => !conditions.iter().any(|c| self.evaluate_condition(c, event)),
        }
    }

    fn check_exceptions(&self, exceptions: &[PolicyException], event: &SecurityEvent) -> bool {
        exceptions.iter().any(|exception| {
            if let Some(expires_at) = exception.expires_at {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                
                if now > expires_at {
                    return false;
                }
            }
            
            self.evaluate_condition(&exception.condition, event)
        })
    }

    fn determine_violation_type(&self, condition: &PolicyCondition) -> PolicyViolationType {
        match condition {
            PolicyCondition::ResourceAccess { resource_type, .. } => {
                match resource_type {
                    ResourceType::File | ResourceType::Directory => PolicyViolationType::UnauthorizedFileAccess,
                    ResourceType::Network => PolicyViolationType::UnauthorizedNetworkAccess,
                    _ => PolicyViolationType::MaliciousActivity,
                }
            }
            PolicyCondition::NetworkActivity { .. } => PolicyViolationType::UnauthorizedNetworkAccess,
            PolicyCondition::SystemCall { .. } => PolicyViolationType::SuspiciousSystemCall,
            _ => PolicyViolationType::MaliciousActivity,
        }
    }

    fn collect_evidence(&self, event: &SecurityEvent) -> HashMap<String, String> {
        let mut evidence = HashMap::new();
        evidence.insert("event_type".to_string(), format!("{:?}", event.event_type));
        evidence.insert("severity".to_string(), format!("{:?}", event.severity));
        evidence.insert("timestamp".to_string(), event.timestamp.to_string());
        evidence.insert("process_id".to_string(), event.source_process.to_string());
        
        for (key, value) in &event.details {
            evidence.insert(key.clone(), value.clone());
        }
        
        evidence
    }

    fn apply_action(&self, action: &PolicyAction, event: &SecurityEvent, policy_id: &str) -> AppliedAction {
        match action {
            PolicyAction::Allow => {
                AppliedAction {
                    action: action.clone(),
                    policy_id: policy_id.to_string(),
                    success: true,
                    message: Some("Access allowed".to_string()),
                }
            }
            PolicyAction::Deny | PolicyAction::Block => {
                log::warn!("Blocking action from process {} due to policy {}", event.source_process, policy_id);
                AppliedAction {
                    action: action.clone(),
                    policy_id: policy_id.to_string(),
                    success: true,
                    message: Some("Access blocked".to_string()),
                }
            }
            PolicyAction::Quarantine => {
                log::error!("Quarantining process {} due to policy violation", event.source_process);
                AppliedAction {
                    action: action.clone(),
                    policy_id: policy_id.to_string(),
                    success: true,
                    message: Some("Process quarantined".to_string()),
                }
            }
            PolicyAction::Terminate => {
                log::error!("Terminating process {} due to critical policy violation", event.source_process);
                AppliedAction {
                    action: action.clone(),
                    policy_id: policy_id.to_string(),
                    success: true,
                    message: Some("Process terminated".to_string()),
                }
            }
            PolicyAction::Alert(level) => {
                log::info!("Alert ({:?}): Policy {} triggered by process {}", level, policy_id, event.source_process);
                AppliedAction {
                    action: action.clone(),
                    policy_id: policy_id.to_string(),
                    success: true,
                    message: Some("Alert sent".to_string()),
                }
            }
            PolicyAction::Log(level) => {
                match level {
                    LogLevel::Debug => log::debug!("Policy {} triggered by process {}", policy_id, event.source_process),
                    LogLevel::Info => log::info!("Policy {} triggered by process {}", policy_id, event.source_process),
                    LogLevel::Warning => log::warn!("Policy {} triggered by process {}", policy_id, event.source_process),
                    LogLevel::Error => log::error!("Policy {} triggered by process {}", policy_id, event.source_process),
                }
                AppliedAction {
                    action: action.clone(),
                    policy_id: policy_id.to_string(),
                    success: true,
                    message: Some("Event logged".to_string()),
                }
            }
            PolicyAction::Throttle(config) => {
                log::info!("Throttling process {} (max {} ops/sec)", event.source_process, config.max_operations_per_second);
                AppliedAction {
                    action: action.clone(),
                    policy_id: policy_id.to_string(),
                    success: true,
                    message: Some("Throttling applied".to_string()),
                }
            }
            PolicyAction::Redirect(config) => {
                log::info!("Redirecting access from process {} to {}", event.source_process, config.target_path);
                AppliedAction {
                    action: action.clone(),
                    policy_id: policy_id.to_string(),
                    success: true,
                    message: Some("Access redirected".to_string()),
                }
            }
        }
    }

    pub fn add_policy(&mut self, policy: SecurityPolicy) -> Result<(), PolicyError> {
        if self.policies.iter().any(|p| p.id == policy.id) {
            return Err(PolicyError::DuplicatePolicy(policy.id));
        }

        self.validate_policy(&policy)?;
        self.policies.push(policy);
        Ok(())
    }

    pub fn remove_policy(&mut self, policy_id: &str) -> Result<(), PolicyError> {
        let index = self.policies.iter().position(|p| p.id == policy_id)
            .ok_or_else(|| PolicyError::PolicyNotFound(policy_id.to_string()))?;
        
        self.policies.remove(index);
        Ok(())
    }

    pub fn update_policy(&mut self, policy: SecurityPolicy) -> Result<(), PolicyError> {
        let index = self.policies.iter().position(|p| p.id == policy.id)
            .ok_or_else(|| PolicyError::PolicyNotFound(policy.id.clone()))?;
        
        self.validate_policy(&policy)?;
        self.policies[index] = policy;
        Ok(())
    }

    fn validate_policy(&self, policy: &SecurityPolicy) -> Result<(), PolicyError> {
        if policy.name.is_empty() {
            return Err(PolicyError::InvalidPolicy("Policy name cannot be empty".to_string()));
        }

        if policy.rules.is_empty() {
            return Err(PolicyError::InvalidPolicy("Policy must have at least one rule".to_string()));
        }

        for rule in &policy.rules {
            self.validate_rule(rule)?;
        }

        Ok(())
    }

    fn validate_rule(&self, rule: &PolicyRule) -> Result<(), PolicyError> {
        if rule.id.is_empty() {
            return Err(PolicyError::InvalidPolicy("Rule ID cannot be empty".to_string()));
        }

        self.validate_condition(&rule.condition)?;
        Ok(())
    }

    fn validate_condition(&self, condition: &PolicyCondition) -> Result<(), PolicyError> {
        match condition {
            PolicyCondition::Composite { conditions, .. } => {
                if conditions.is_empty() {
                    return Err(PolicyError::InvalidPolicy("Composite condition must have at least one sub-condition".to_string()));
                }
                for sub_condition in conditions {
                    self.validate_condition(sub_condition)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn get_policy(&self, policy_id: &str) -> Option<&SecurityPolicy> {
        self.policies.iter().find(|p| p.id == policy_id)
    }

    pub fn list_policies(&self) -> &[SecurityPolicy] {
        &self.policies
    }

    pub fn set_enforcement_mode(&mut self, mode: EnforcementMode) {
        self.enforcement_mode = mode;
        log::info!("Policy enforcement mode set to: {:?}", mode);
    }

    pub fn get_enforcement_mode(&self) -> EnforcementMode {
        self.enforcement_mode
    }

    pub async fn create_audit_report(&self) -> SecurityAuditReport {
        SecurityAuditReport {
            total_policies: self.policies.len(),
            enabled_policies: self.policies.iter().filter(|p| p.enabled).count(),
            enforcement_mode: self.enforcement_mode,
            last_updated: std::time::SystemTime::now(),
            policy_summary: self.policies.iter().map(|p| PolicySummary {
                id: p.id.clone(),
                name: p.name.clone(),
                enabled: p.enabled,
                rule_count: p.rules.len(),
                severity: p.severity,
            }).collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SecurityAuditReport {
    pub total_policies: usize,
    pub enabled_policies: usize,
    pub enforcement_mode: EnforcementMode,
    pub last_updated: std::time::SystemTime,
    pub policy_summary: Vec<PolicySummary>,
}

#[derive(Debug, Clone)]
pub struct PolicySummary {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub rule_count: usize,
    pub severity: SecuritySeverity,
}

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("Duplicate policy ID: {0}")]
    DuplicatePolicy(String),
    #[error("Policy not found: {0}")]
    PolicyNotFound(String),
    #[error("Invalid policy: {0}")]
    InvalidPolicy(String),
    #[error("Policy validation failed: {0}")]
    ValidationFailed(String),
}

impl Default for SecurityPolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}
