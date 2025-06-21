use super::{ProcessConfig, ProcessError, ProcessStats, ProcessStatus, SandboxedProcess};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, Duration};
use tracing::log;

pub struct ProcessManager {
    processes: Arc<RwLock<HashMap<u32, SandboxedProcess>>>,
    next_process_id: Arc<RwLock<u32>>,
    resource_monitor: ResourceMonitor,
    cleanup_scheduler: CleanupScheduler,
    process_events: mpsc::UnboundedSender<ProcessEvent>,
}

#[derive(Debug, Clone)]
pub enum ProcessEvent {
    ProcessStarted(u32),
    ProcessTerminated(u32, Option<i32>),
    ProcessFailed(u32, String),
    ResourceLimitExceeded(u32, String),
    HealthCheckFailed(u32),
}

struct ResourceMonitor {
    monitoring_interval: Duration,
    alert_thresholds: ResourceThresholds,
    event_sender: mpsc::UnboundedSender<ProcessEvent>,
}

struct CleanupScheduler {
    cleanup_interval: Duration,
    max_terminated_age: Duration,
    event_sender: mpsc::UnboundedSender<ProcessEvent>,
}

#[derive(Debug, Clone)]
struct ResourceThresholds {
    memory_warning_percent: f64,
    cpu_warning_percent: f64,
    file_handle_warning_percent: f64,
}

impl ProcessManager {
    pub fn new() -> Self {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        
        let manager = Self {
            processes: Arc::new(RwLock::new(HashMap::new())),
            next_process_id: Arc::new(RwLock::new(1)),
            resource_monitor: ResourceMonitor::new(event_sender.clone()),
            cleanup_scheduler: CleanupScheduler::new(event_sender.clone()),
            process_events: event_sender,
        };

        manager.start_background_tasks();
        manager.start_event_handler(event_receiver);
        manager
    }

    pub async fn create_process(&self, config: ProcessConfig) -> Result<u32, ProcessError> {
        let process_id = self.generate_process_id().await;
        let process = SandboxedProcess::new(process_id, config).await?;
        
        {
            let mut processes = self.processes.write().await;
            processes.insert(process_id, process);
        }

        self.process_events.send(ProcessEvent::ProcessStarted(process_id))
            .map_err(|_| ProcessError::CommandSendFailed)?;

        log::info!("Created process with ID: {}", process_id);
        Ok(process_id)
    }

    pub async fn start_process(&self, process_id: u32) -> Result<(), ProcessError> {
        let mut processes = self.processes.write().await;
        if let Some(process) = processes.get_mut(&process_id) {
            process.start().await?;
            log::info!("Started process: {}", process_id);
            Ok(())
        } else {
            Err(ProcessError::InvalidPid)
        }
    }

    pub async fn terminate_process(&self, process_id: u32) -> Result<(), ProcessError> {
        let mut processes = self.processes.write().await;
        if let Some(process) = processes.get_mut(&process_id) {
            process.terminate().await?;
            
            let stats = process.get_stats().await;
            self.process_events.send(ProcessEvent::ProcessTerminated(process_id, stats.exit_code))
                .map_err(|_| ProcessError::CommandSendFailed)?;
            
            log::info!("Terminated process: {}", process_id);
            Ok(())
        } else {
            Err(ProcessError::InvalidPid)
        }
    }

    pub async fn kill_process(&self, process_id: u32) -> Result<(), ProcessError> {
        let mut processes = self.processes.write().await;
        if let Some(process) = processes.remove(&process_id) {
            drop(process);
            
            self.process_events.send(ProcessEvent::ProcessTerminated(process_id, Some(-9)))
                .map_err(|_| ProcessError::CommandSendFailed)?;
            
            log::info!("Killed process: {}", process_id);
            Ok(())
        } else {
            Err(ProcessError::InvalidPid)
        }
    }

    pub async fn suspend_process(&self, process_id: u32) -> Result<(), ProcessError> {
        let mut processes = self.processes.write().await;
        if let Some(process) = processes.get_mut(&process_id) {
            process.suspend().await?;
            log::info!("Suspended process: {}", process_id);
            Ok(())
        } else {
            Err(ProcessError::InvalidPid)
        }
    }

    pub async fn resume_process(&self, process_id: u32) -> Result<(), ProcessError> {
        let mut processes = self.processes.write().await;
        if let Some(process) = processes.get_mut(&process_id) {
            process.resume().await?;
            log::info!("Resumed process: {}", process_id);
            Ok(())
        } else {
            Err(ProcessError::InvalidPid)
        }
    }

    pub async fn get_process_stats(&self, process_id: u32) -> Option<ProcessStats> {
        let processes = self.processes.read().await;
        if let Some(process) = processes.get(&process_id) {
            Some(process.get_stats().await)
        } else {
            None
        }
    }

    pub async fn get_process_status(&self, process_id: u32) -> Option<ProcessStatus> {
        let processes = self.processes.read().await;
        if let Some(process) = processes.get(&process_id) {
            Some(process.get_status().await)
        } else {
            None
        }
    }

    pub async fn list_processes(&self) -> Vec<ProcessSummary> {
        let processes = self.processes.read().await;
        let mut summaries = Vec::new();
        
        for (id, process) in processes.iter() {
            let stats = process.get_stats().await;
            let status = process.get_status().await;
            
            summaries.push(ProcessSummary {
                id: *id,
                status,
                memory_usage: stats.memory_usage_bytes,
                cpu_usage: stats.cpu_usage_percent,
                execution_time: stats.execution_time,
                exit_code: stats.exit_code,
            });
        }
        
        summaries
    }

    pub async fn get_system_resources(&self) -> SystemResourceUsage {
        let processes = self.processes.read().await;
        let mut total_memory = 0;
        let mut total_cpu = 0.0;
        let mut total_processes = 0;
        let mut running_processes = 0;
        
        for process in processes.values() {
            let stats = process.get_stats().await;
            let status = process.get_status().await;
            
            total_memory += stats.memory_usage_bytes;
            total_cpu += stats.cpu_usage_percent;
            total_processes += 1;
            
            if status == ProcessStatus::Running {
                running_processes += 1;
            }
        }

        SystemResourceUsage {
            total_memory_bytes: total_memory,
            average_cpu_percent: if total_processes > 0 { total_cpu / total_processes as f64 } else { 0.0 },
            total_processes,
            running_processes,
            system_load: Self::get_system_load().await,
        }
    }

    async fn get_system_load() -> f64 {
        #[cfg(target_os = "linux")]
        {
            if let Ok(loadavg) = tokio::fs::read_to_string("/proc/loadavg").await {
                if let Some(first_value) = loadavg.split_whitespace().next() {
                    return first_value.parse().unwrap_or(0.0);
                }
            }
        }
        0.0
    }

    pub async fn health_check(&self) -> HealthStatus {
        let processes = self.processes.read().await;
        let mut healthy = 0;
        let mut unhealthy = 0;
        let mut failed_checks = Vec::new();

        for (id, process) in processes.iter() {
            let status = process.get_status().await;
            match status {
                ProcessStatus::Running => healthy += 1,
                ProcessStatus::Failed => {
                    unhealthy += 1;
                    failed_checks.push(*id);
                }
                ProcessStatus::Terminated => {},
                _ => unhealthy += 1,
            }
        }

        let overall_status = if unhealthy == 0 {
            ServiceHealth::Healthy
        } else if unhealthy < healthy {
            ServiceHealth::Degraded
        } else {
            ServiceHealth::Unhealthy
        };

        HealthStatus {
            overall_status,
            healthy_processes: healthy,
            unhealthy_processes: unhealthy,
            failed_process_ids: failed_checks,
            last_check: std::time::Instant::now(),
        }
    }

    async fn generate_process_id(&self) -> u32 {
        let mut next_id = self.next_process_id.write().await;
        let id = *next_id;
        *next_id += 1;
        id
    }

    fn start_background_tasks(&self) {
        self.resource_monitor.start(self.processes.clone());
        self.cleanup_scheduler.start(self.processes.clone());
    }

    fn start_event_handler(&self, mut receiver: mpsc::UnboundedReceiver<ProcessEvent>) {
        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                match event {
                    ProcessEvent::ProcessStarted(id) => {
                        log::info!("Process {} started successfully", id);
                    }
                    ProcessEvent::ProcessTerminated(id, code) => {
                        log::info!("Process {} terminated with code: {:?}", id, code);
                    }
                    ProcessEvent::ProcessFailed(id, reason) => {
                        log::error!("Process {} failed: {}", id, reason);
                    }
                    ProcessEvent::ResourceLimitExceeded(id, resource) => {
                        log::warn!("Process {} exceeded {} limit", id, resource);
                    }
                    ProcessEvent::HealthCheckFailed(id) => {
                        log::warn!("Health check failed for process {}", id);
                    }
                }
            }
        });
    }

    pub async fn force_cleanup(&self) -> Result<usize, ProcessError> {
        let mut processes = self.processes.write().await;
        let mut cleaned_count = 0;
        
        let terminated_ids: Vec<u32> = processes.iter()
            .filter_map(|(id, process)| {
                let status = futures::executor::block_on(process.get_status());
                if matches!(status, ProcessStatus::Terminated | ProcessStatus::Failed) {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();

        for id in terminated_ids {
            processes.remove(&id);
            cleaned_count += 1;
        }

        log::info!("Force cleanup removed {} terminated processes", cleaned_count);
        Ok(cleaned_count)
    }
}

impl ResourceMonitor {
    fn new(event_sender: mpsc::UnboundedSender<ProcessEvent>) -> Self {
        Self {
            monitoring_interval: Duration::from_secs(5),
            alert_thresholds: ResourceThresholds {
                memory_warning_percent: 90.0,
                cpu_warning_percent: 95.0,
                file_handle_warning_percent: 85.0,
            },
            event_sender,
        }
    }

    fn start(&self, processes: Arc<RwLock<HashMap<u32, SandboxedProcess>>>) {
        let monitoring_interval = self.monitoring_interval;
        let thresholds = self.alert_thresholds.clone();
        let event_sender = self.event_sender.clone();

        tokio::spawn(async move {
            let mut interval_timer = interval(monitoring_interval);

            loop {
                interval_timer.tick().await;
                
                let processes_guard = processes.read().await;
                for (id, process) in processes_guard.iter() {
                    let stats = process.get_stats().await;
                    let config = &process.config;
                    
                    let memory_percent = (stats.memory_usage_bytes as f64 / (config.resource_limits.max_memory_mb * 1024 * 1024) as f64) * 100.0;
                    let cpu_percent = stats.cpu_usage_percent;

                    if memory_percent > thresholds.memory_warning_percent {
                        let _ = event_sender.send(ProcessEvent::ResourceLimitExceeded(*id, "memory".to_string()));
                    }

                    if cpu_percent > thresholds.cpu_warning_percent {
                        let _ = event_sender.send(ProcessEvent::ResourceLimitExceeded(*id, "cpu".to_string()));
                    }
                }
            }
        });
    }
}

impl CleanupScheduler {
    fn new(event_sender: mpsc::UnboundedSender<ProcessEvent>) -> Self {
        Self {
            cleanup_interval: Duration::from_secs(300),
            max_terminated_age: Duration::from_secs(3600),
            event_sender,
        }
    }

    fn start(&self, processes: Arc<RwLock<HashMap<u32, SandboxedProcess>>>) {
        let cleanup_interval = self.cleanup_interval;
        let max_age = self.max_terminated_age;

        tokio::spawn(async move {
            let mut interval_timer = interval(cleanup_interval);

            loop {
                interval_timer.tick().await;
                
                let mut processes_guard = processes.write().await;
                let now = std::time::Instant::now();
                
                let to_remove: Vec<u32> = processes_guard.iter()
                    .filter_map(|(id, process)| {
                        let status = futures::executor::block_on(process.get_status());
                        let stats = futures::executor::block_on(process.get_stats());
                        
                        if matches!(status, ProcessStatus::Terminated | ProcessStatus::Failed) {
                            if let Some(start_time) = stats.start_time {
                                if now.duration_since(start_time) > max_age {
                                    return Some(*id);
                                }
                            }
                        }
                        None
                    })
                    .collect();

                for id in to_remove {
                    processes_guard.remove(&id);
                    log::debug!("Cleaned up terminated process: {}", id);
                }
            }
        });
    }
}

#[derive(Debug, Clone)]
pub struct ProcessSummary {
    pub id: u32,
    pub status: ProcessStatus,
    pub memory_usage: u64,
    pub cpu_usage: f64,
    pub execution_time: std::time::Duration,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct SystemResourceUsage {
    pub total_memory_bytes: u64,
    pub average_cpu_percent: f64,
    pub total_processes: u32,
    pub running_processes: u32,
    pub system_load: f64,
}

#[derive(Debug, Clone)]
pub struct HealthStatus {
    pub overall_status: ServiceHealth,
    pub healthy_processes: u32,
    pub unhealthy_processes: u32,
    pub failed_process_ids: Vec<u32>,
    pub last_check: std::time::Instant,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ServiceHealth {
    Healthy,
    Degraded,
    Unhealthy,
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new()
    }
}