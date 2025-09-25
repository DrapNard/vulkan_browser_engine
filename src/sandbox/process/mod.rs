pub mod manager;

pub use manager::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, RwLock};
use tracing::log;

pub struct SandboxedProcess {
    pub id: u32,
    pub config: ProcessConfig,
    pub handle: Option<Child>,
    pub stats: Arc<RwLock<ProcessStats>>,
    pub command_sender: mpsc::UnboundedSender<ProcessCommand>,
    pub status: Arc<RwLock<ProcessStatus>>,
    isolation_manager: IsolationManager,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessConfig {
    pub executable_path: String,
    pub arguments: Vec<String>,
    pub environment: HashMap<String, String>,
    pub working_directory: Option<String>,
    pub isolation_level: IsolationLevel,
    pub resource_limits: ResourceLimits,
    pub network_restrictions: NetworkRestrictions,
    pub file_system_restrictions: FileSystemRestrictions,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum IsolationLevel {
    None,
    Basic,
    Strict,
    Maximum,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub max_memory_mb: u64,
    pub max_cpu_percent: u8,
    pub max_file_handles: u32,
    pub max_threads: u32,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRestrictions {
    pub allow_network: bool,
    pub allowed_domains: Vec<String>,
    pub blocked_ports: Vec<u16>,
    pub bandwidth_limit_kbps: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSystemRestrictions {
    pub read_only_paths: Vec<String>,
    pub writable_paths: Vec<String>,
    pub blocked_paths: Vec<String>,
    pub temp_directory: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ProcessStats {
    pub pid: Option<u32>,
    pub memory_usage_bytes: u64,
    pub cpu_usage_percent: f64,
    pub file_handles_count: u32,
    pub thread_count: u32,
    pub network_bytes_sent: u64,
    pub network_bytes_received: u64,
    pub start_time: Option<std::time::Instant>,
    pub execution_time: std::time::Duration,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProcessStatus {
    Starting,
    Running,
    Suspended,
    Terminating,
    Terminated,
    Failed,
}

#[derive(Debug, Clone)]
pub enum ProcessCommand {
    Start,
    Stop,
    Suspend,
    Resume,
    UpdateLimits(ResourceLimits),
    GetStats,
}

struct IsolationManager {
    isolation_level: IsolationLevel,
    namespace_handle: Option<NamespaceHandle>,
    seccomp_filter: Option<SeccompFilter>,
    capabilities: Vec<String>,
}

struct NamespaceHandle {
    pid_namespace: Option<String>,
    network_namespace: Option<String>,
    mount_namespace: Option<String>,
    user_namespace: Option<String>,
}

struct SeccompFilter {
    allowed_syscalls: Vec<String>,
    blocked_syscalls: Vec<String>,
    default_action: SeccompAction,
}

#[derive(Debug, Clone)]
enum SeccompAction {
    Allow,
    Kill,
    Trap,
    Errno(i32),
}

impl SandboxedProcess {
    pub async fn new(id: u32, config: ProcessConfig) -> Result<Self, ProcessError> {
        let (command_sender, command_receiver) = mpsc::unbounded_channel();
        let stats = Arc::new(RwLock::new(ProcessStats::default()));
        let status = Arc::new(RwLock::new(ProcessStatus::Starting));
        let isolation_manager = IsolationManager::new(config.isolation_level).await?;

        let process = Self {
            id,
            config,
            handle: None,
            stats: stats.clone(),
            command_sender,
            status: status.clone(),
            isolation_manager,
        };

        Self::spawn_command_handler(id, command_receiver, stats, status).await;
        Ok(process)
    }

    pub async fn start(&mut self) -> Result<(), ProcessError> {
        {
            let mut status = self.status.write().await;
            *status = ProcessStatus::Starting;
        }

        let mut command = Command::new(&self.config.executable_path);
        command.args(&self.config.arguments);

        for (key, value) in &self.config.environment {
            command.env(key, value);
        }

        if let Some(ref wd) = self.config.working_directory {
            command.current_dir(wd);
        }

        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        self.isolation_manager
            .apply_restrictions(&mut command)
            .await?;

        let child = command
            .spawn()
            .map_err(|e| ProcessError::SpawnFailed(e.to_string()))?;

        let pid = child.id().ok_or(ProcessError::InvalidPid)?;

        {
            let mut stats = self.stats.write().await;
            stats.pid = Some(pid);
            stats.start_time = Some(std::time::Instant::now());
        }

        self.handle = Some(child);

        {
            let mut status = self.status.write().await;
            *status = ProcessStatus::Running;
        }

        self.start_monitoring().await;

        Ok(())
    }

    pub async fn terminate(&mut self) -> Result<(), ProcessError> {
        {
            let mut status = self.status.write().await;
            *status = ProcessStatus::Terminating;
        }

        if let Some(ref mut child) = self.handle {
            child
                .kill()
                .await
                .map_err(|e| ProcessError::TerminationFailed(e.to_string()))?;

            let exit_status = child
                .wait()
                .await
                .map_err(|e| ProcessError::WaitFailed(e.to_string()))?;

            {
                let mut stats = self.stats.write().await;
                stats.exit_code = exit_status.code();
            }
        }

        {
            let mut status = self.status.write().await;
            *status = ProcessStatus::Terminated;
        }

        Ok(())
    }

    pub async fn suspend(&mut self) -> Result<(), ProcessError> {
        if let Some(ref child) = self.handle {
            let _pid = child.id().ok_or(ProcessError::InvalidPid)?;

            #[cfg(unix)]
            unsafe {
                libc::kill(_pid as i32, libc::SIGSTOP);
            }

            let mut status = self.status.write().await;
            *status = ProcessStatus::Suspended;
        }
        Ok(())
    }

    pub async fn resume(&mut self) -> Result<(), ProcessError> {
        if let Some(ref child) = self.handle {
            let _pid = child.id().ok_or(ProcessError::InvalidPid)?;

            #[cfg(unix)]
            unsafe {
                libc::kill(_pid as i32, libc::SIGCONT);
            }

            let mut status = self.status.write().await;
            *status = ProcessStatus::Running;
        }
        Ok(())
    }

    async fn start_monitoring(&self) {
        let stats = self.stats.clone();
        let status = self.status.clone();
        let pid = {
            let stats_guard = stats.read().await;
            stats_guard.pid
        };

        if let Some(pid) = pid {
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));

                loop {
                    interval.tick().await;

                    let current_status = *status.read().await;
                    if matches!(
                        current_status,
                        ProcessStatus::Terminated | ProcessStatus::Failed
                    ) {
                        break;
                    }

                    if let Ok(process_info) = Self::get_process_info(pid).await {
                        let mut stats_guard = stats.write().await;
                        stats_guard.memory_usage_bytes = process_info.memory_bytes;
                        stats_guard.cpu_usage_percent = process_info.cpu_percent;
                        stats_guard.thread_count = process_info.thread_count;

                        if let Some(start_time) = stats_guard.start_time {
                            stats_guard.execution_time = start_time.elapsed();
                        }
                    }
                }
            });
        }
    }

    async fn get_process_info(_pid: u32) -> Result<ProcessInfo, ProcessError> {
        #[cfg(target_os = "linux")]
        {
            let stat_path = format!("/proc/{}/stat", _pid);
            let status_path = format!("/proc/{}/status", _pid);

            let stat_content = tokio::fs::read_to_string(stat_path)
                .await
                .map_err(|_| ProcessError::MonitoringFailed)?;
            let status_content = tokio::fs::read_to_string(status_path)
                .await
                .map_err(|_| ProcessError::MonitoringFailed)?;

            Ok(ProcessInfo::parse_linux_proc(stat_content, status_content)?)
        }

        #[cfg(not(target_os = "linux"))]
        {
            Ok(ProcessInfo {
                memory_bytes: 0,
                cpu_percent: 0.0,
                thread_count: 1,
            })
        }
    }

    async fn spawn_command_handler(
        process_id: u32,
        mut receiver: mpsc::UnboundedReceiver<ProcessCommand>,
        stats: Arc<RwLock<ProcessStats>>,
        _status: Arc<RwLock<ProcessStatus>>,
    ) {
        tokio::spawn(async move {
            while let Some(command) = receiver.recv().await {
                match command {
                    ProcessCommand::GetStats => {
                        let stats_guard = stats.read().await;
                        log::debug!("Process {} stats: {:?}", process_id, *stats_guard);
                    }
                    ProcessCommand::UpdateLimits(_new_limits) => {
                        log::info!("Updating resource limits for process {}", process_id);
                    }
                    _ => {
                        log::debug!("Received command for process {}: {:?}", process_id, command);
                    }
                }
            }
        });
    }

    pub async fn get_stats(&self) -> ProcessStats {
        self.stats.read().await.clone()
    }

    pub async fn get_status(&self) -> ProcessStatus {
        *self.status.read().await
    }

    pub fn send_command(&self, command: ProcessCommand) -> Result<(), ProcessError> {
        self.command_sender
            .send(command)
            .map_err(|_| ProcessError::CommandSendFailed)
    }
}

impl IsolationManager {
    async fn new(isolation_level: IsolationLevel) -> Result<Self, ProcessError> {
        let namespace_handle = match isolation_level {
            IsolationLevel::Maximum | IsolationLevel::Strict => {
                Some(NamespaceHandle::create().await?)
            }
            _ => None,
        };

        let seccomp_filter = match isolation_level {
            IsolationLevel::Maximum => Some(SeccompFilter::strict()),
            IsolationLevel::Strict => Some(SeccompFilter::moderate()),
            _ => None,
        };

        let capabilities = match isolation_level {
            IsolationLevel::Maximum => vec![],
            IsolationLevel::Strict => vec!["CAP_NET_RAW".to_string()],
            IsolationLevel::Basic => vec!["CAP_NET_RAW".to_string(), "CAP_SYS_NICE".to_string()],
            IsolationLevel::None => vec!["CAP_SYS_ADMIN".to_string()],
        };

        Ok(Self {
            isolation_level,
            namespace_handle,
            seccomp_filter,
            capabilities,
        })
    }

    async fn apply_restrictions(&self, _command: &mut Command) -> Result<(), ProcessError> {
        match self.isolation_level {
            IsolationLevel::Maximum | IsolationLevel::Strict => {
                #[cfg(target_os = "linux")]
                {
                    _command.env("SECCOMP", "1");
                    _command.env("UNSHARE_NET", "1");
                    _command.env("UNSHARE_PID", "1");
                }
            }
            _ => {}
        }

        Ok(())
    }
}

impl NamespaceHandle {
    async fn create() -> Result<Self, ProcessError> {
        Ok(Self {
            pid_namespace: Some("isolated_pid".to_string()),
            network_namespace: Some("isolated_net".to_string()),
            mount_namespace: Some("isolated_mount".to_string()),
            user_namespace: Some("isolated_user".to_string()),
        })
    }
}

impl SeccompFilter {
    fn strict() -> Self {
        Self {
            allowed_syscalls: vec![
                "read".to_string(),
                "write".to_string(),
                "exit".to_string(),
                "exit_group".to_string(),
                "mmap".to_string(),
                "munmap".to_string(),
            ],
            blocked_syscalls: vec![
                "execve".to_string(),
                "fork".to_string(),
                "clone".to_string(),
                "ptrace".to_string(),
            ],
            default_action: SeccompAction::Kill,
        }
    }

    fn moderate() -> Self {
        Self {
            allowed_syscalls: vec![
                "read".to_string(),
                "write".to_string(),
                "open".to_string(),
                "close".to_string(),
                "stat".to_string(),
                "fstat".to_string(),
                "mmap".to_string(),
                "munmap".to_string(),
                "brk".to_string(),
                "rt_sigaction".to_string(),
                "rt_sigprocmask".to_string(),
                "ioctl".to_string(),
                "access".to_string(),
                "exit".to_string(),
                "exit_group".to_string(),
            ],
            blocked_syscalls: vec![
                "ptrace".to_string(),
                "setuid".to_string(),
                "setgid".to_string(),
            ],
            default_action: SeccompAction::Errno(1),
        }
    }
}

struct ProcessInfo {
    memory_bytes: u64,
    cpu_percent: f64,
    thread_count: u32,
}

impl ProcessInfo {
    #[cfg(target_os = "linux")]
    fn parse_linux_proc(
        stat_content: String,
        status_content: String,
    ) -> Result<Self, ProcessError> {
        let mut memory_bytes = 0;
        let mut thread_count = 1;

        for line in status_content.lines() {
            if line.starts_with("VmRSS:") {
                if let Some(value) = line.split_whitespace().nth(1) {
                    memory_bytes = value.parse::<u64>().unwrap_or(0) * 1024;
                }
            } else if line.starts_with("Threads:") {
                if let Some(value) = line.split_whitespace().nth(1) {
                    thread_count = value.parse::<u32>().unwrap_or(1);
                }
            }
        }

        Ok(ProcessInfo {
            memory_bytes,
            cpu_percent: 0.0,
            thread_count,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("Failed to spawn process: {0}")]
    SpawnFailed(String),
    #[error("Invalid process ID")]
    InvalidPid,
    #[error("Process termination failed: {0}")]
    TerminationFailed(String),
    #[error("Failed to wait for process: {0}")]
    WaitFailed(String),
    #[error("Failed to send command")]
    CommandSendFailed,
    #[error("Monitoring failed")]
    MonitoringFailed,
    #[error("Isolation setup failed: {0}")]
    IsolationFailed(String),
    #[error("Resource limit enforcement failed: {0}")]
    ResourceLimitFailed(String),
}

impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            executable_path: String::new(),
            arguments: Vec::new(),
            environment: HashMap::new(),
            working_directory: None,
            isolation_level: IsolationLevel::Basic,
            resource_limits: ResourceLimits::default(),
            network_restrictions: NetworkRestrictions::default(),
            file_system_restrictions: FileSystemRestrictions::default(),
        }
    }
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory_mb: 512,
            max_cpu_percent: 80,
            max_file_handles: 1024,
            max_threads: 100,
            timeout_seconds: 300,
        }
    }
}

impl Default for NetworkRestrictions {
    fn default() -> Self {
        Self {
            allow_network: true,
            allowed_domains: Vec::new(),
            blocked_ports: vec![22, 23, 135, 139, 445],
            bandwidth_limit_kbps: None,
        }
    }
}

impl Default for FileSystemRestrictions {
    fn default() -> Self {
        Self {
            read_only_paths: vec!["/usr".to_string(), "/bin".to_string()],
            writable_paths: vec!["/tmp".to_string()],
            blocked_paths: vec!["/etc/passwd".to_string(), "/etc/shadow".to_string()],
            temp_directory: Some("/tmp".to_string()),
        }
    }
}
