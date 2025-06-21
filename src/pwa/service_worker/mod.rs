pub mod runtime;

pub use runtime::*;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ServiceWorkerManager {
    workers: Arc<RwLock<HashMap<String, ServiceWorker>>>,
    runtime: ServiceWorkerRuntime,
}

pub struct ServiceWorker {
    pub id: String,
    pub script_url: String,
    pub scope: String,
    pub state: ServiceWorkerState,
    pub installation_time: std::time::SystemTime,
    pub last_update_check: std::time::SystemTime,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServiceWorkerState {
    Installing,
    Installed,
    Activating,
    Activated,
    Redundant,
}

impl ServiceWorkerManager {
    pub fn new() -> Self {
        Self {
            workers: Arc::new(RwLock::new(HashMap::new())),
            runtime: ServiceWorkerRuntime::new(),
        }
    }

    pub async fn register(&mut self, script_url: &str, scope: &str) -> Result<String, ServiceWorkerError> {
        let worker_id = self.generate_worker_id(script_url, scope);
        
        let worker = ServiceWorker {
            id: worker_id.clone(),
            script_url: script_url.to_string(),
            scope: scope.to_string(),
            state: ServiceWorkerState::Installing,
            installation_time: std::time::SystemTime::now(),
            last_update_check: std::time::SystemTime::now(),
        };

        {
            let mut workers = self.workers.write().await;
            workers.insert(worker_id.clone(), worker);
        }

        self.runtime.install_worker(script_url, scope).await?;
        
        {
            let mut workers = self.workers.write().await;
            if let Some(worker) = workers.get_mut(&worker_id) {
                worker.state = ServiceWorkerState::Installed;
            }
        }

        self.runtime.activate_worker(&worker_id).await?;

        {
            let mut workers = self.workers.write().await;
            if let Some(worker) = workers.get_mut(&worker_id) {
                worker.state = ServiceWorkerState::Activated;
            }
        }

        log::info!("Service Worker registered: {} ({})", script_url, worker_id);
        Ok(worker_id)
    }

    pub async fn unregister(&mut self, worker_id: &str) -> Result<(), ServiceWorkerError> {
        {
            let mut workers = self.workers.write().await;
            if let Some(worker) = workers.get_mut(worker_id) {
                worker.state = ServiceWorkerState::Redundant;
            }
        }

        self.runtime.terminate_worker(worker_id).await?;

        {
            let mut workers = self.workers.write().await;
            workers.remove(worker_id);
        }

        log::info!("Service Worker unregistered: {}", worker_id);
        Ok(())
    }

    pub async fn update(&mut self, worker_id: &str) -> Result<(), ServiceWorkerError> {
        let (script_url, scope) = {
            let workers = self.workers.read().await;
            if let Some(worker) = workers.get(worker_id) {
                (worker.script_url.clone(), worker.scope.clone())
            } else {
                return Err(ServiceWorkerError::WorkerNotFound(worker_id.to_string()));
            }
        };

        self.runtime.update_worker(&script_url, &scope).await?;

        {
            let mut workers = self.workers.write().await;
            if let Some(worker) = workers.get_mut(worker_id) {
                worker.last_update_check = std::time::SystemTime::now();
            }
        }

        Ok(())
    }

    pub async fn get_registration(&self, scope: &str) -> Option<ServiceWorker> {
        let workers = self.workers.read().await;
        workers.values()
            .find(|worker| worker.scope == scope && worker.state == ServiceWorkerState::Activated)
            .cloned()
    }

    pub async fn handle_fetch(&self, request: &crate::pwa::FetchRequest) -> Result<Option<crate::pwa::FetchResponse>, ServiceWorkerError> {
        let workers = self.workers.read().await;
        for worker in workers.values() {
            if worker.state == ServiceWorkerState::Activated && request.url.starts_with(&worker.scope) {
                return self.runtime.handle_fetch_event(worker, request).await;
            }
        }
        Ok(None)
    }

    fn generate_worker_id(&self, script_url: &str, scope: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        script_url.hash(&mut hasher);
        scope.hash(&mut hasher);
        format!("sw_{:x}", hasher.finish())
    }

    pub async fn get_all_registrations(&self) -> Vec<ServiceWorker> {
        let workers = self.workers.read().await;
        workers.values().cloned().collect()
    }

    pub async fn cleanup_redundant_workers(&mut self) -> Result<(), ServiceWorkerError> {
        let redundant_workers: Vec<String> = {
            let workers = self.workers.read().await;
            workers.iter()
                .filter(|(_, worker)| worker.state == ServiceWorkerState::Redundant)
                .map(|(id, _)| id.clone())
                .collect()
        };

        for worker_id in redundant_workers {
            self.unregister(&worker_id).await?;
        }

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceWorkerError {
    #[error("Worker not found: {0}")]
    WorkerNotFound(String),
    #[error("Installation failed: {0}")]
    InstallationFailed(String),
    #[error("Activation failed: {0}")]
    ActivationFailed(String),
    #[error("Execution error: {0}")]
    ExecutionError(String),
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Script error: {0}")]
    ScriptError(String),
}

impl Default for ServiceWorkerManager {
    fn default() -> Self {
        Self::new()
    }
}