pub mod runtime;

pub use runtime::*;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

pub struct ServiceWorkerManager {
    workers: Arc<RwLock<HashMap<String, ServiceWorker>>>,
    runtime: ServiceWorkerRuntime,
}

#[derive(Debug, Clone)]
pub struct ServiceWorker {
    pub id: String,
    pub script_url: String,
    pub scope: String,
    pub state: ServiceWorkerState,
    pub installation_time: SystemTime,
    pub last_update_check: SystemTime,
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
    pub async fn new() -> Result<Self, ServiceWorkerError> {
        Self::with_config(ServiceWorkerConfig::default()).await
    }

    pub async fn with_config(config: ServiceWorkerConfig) -> Result<Self, ServiceWorkerError> {
        let runtime = ServiceWorkerRuntime::with_config(config).await?;

        Ok(Self {
            workers: Arc::new(RwLock::new(HashMap::new())),
            runtime,
        })
    }

    pub async fn register(
        &self,
        script_url: &str,
        scope: &str,
    ) -> Result<String, ServiceWorkerError> {
        let worker_id = self.generate_worker_id(script_url, scope);

        let worker = ServiceWorker {
            id: worker_id.clone(),
            script_url: script_url.to_string(),
            scope: scope.to_string(),
            state: ServiceWorkerState::Installing,
            installation_time: SystemTime::now(),
            last_update_check: SystemTime::now(),
        };

        self.insert_worker(worker_id.clone(), worker).await;

        match self.runtime.install_worker(script_url, scope).await {
            Ok(runtime_worker_id) => {
                self.update_worker_state(&worker_id, ServiceWorkerState::Installed)
                    .await;

                match self.runtime.activate_worker(&runtime_worker_id).await {
                    Ok(()) => {
                        self.update_worker_state(&worker_id, ServiceWorkerState::Activated)
                            .await;
                        info!(
                            "Service Worker registered successfully: {} ({})",
                            script_url, worker_id
                        );
                        Ok(worker_id)
                    }
                    Err(e) => {
                        self.cleanup_failed_worker(&worker_id).await;
                        error!("Failed to activate worker {}: {}", worker_id, e);
                        Err(e)
                    }
                }
            }
            Err(e) => {
                self.cleanup_failed_worker(&worker_id).await;
                error!("Failed to install worker {}: {}", worker_id, e);
                Err(e)
            }
        }
    }

    pub async fn unregister(&self, worker_id: &str) -> Result<(), ServiceWorkerError> {
        self.update_worker_state(worker_id, ServiceWorkerState::Redundant)
            .await;
        self.runtime.terminate_worker(worker_id).await?;
        self.remove_worker(worker_id).await;
        info!("Service Worker unregistered: {}", worker_id);
        Ok(())
    }

    pub async fn update_worker(&self, worker_id: &str) -> Result<(), ServiceWorkerError> {
        let worker_info = self
            .get_worker_info(worker_id)
            .await
            .ok_or_else(|| ServiceWorkerError::WorkerNotFound(worker_id.to_string()))?;

        let (script_url, scope) = worker_info;

        self.runtime.terminate_worker(worker_id).await?;

        let new_worker_id = self.runtime.install_worker(&script_url, &scope).await?;
        self.runtime.activate_worker(&new_worker_id).await?;

        self.update_worker_timestamp(worker_id).await;

        info!("Service Worker updated: {}", worker_id);
        Ok(())
    }

    pub async fn get_registration(&self, scope: &str) -> Option<ServiceWorker> {
        let workers = self.workers.read().await;
        workers
            .values()
            .find(|worker| worker.scope == scope && worker.state == ServiceWorkerState::Activated)
            .cloned()
    }

    pub async fn handle_fetch(
        &self,
        request: &crate::pwa::FetchRequest,
    ) -> Result<Option<crate::pwa::FetchResponse>, ServiceWorkerError> {
        let matching_worker_id = self.find_matching_worker(&request.url).await;

        if let Some(worker_id) = matching_worker_id {
            self.runtime.handle_fetch_event(&worker_id, request).await
        } else {
            Ok(None)
        }
    }

    pub async fn get_all_registrations(&self) -> Vec<ServiceWorker> {
        let workers = self.workers.read().await;
        workers.values().cloned().collect()
    }

    pub async fn cleanup_redundant_workers(&self) -> Result<usize, ServiceWorkerError> {
        let redundant_worker_ids = self.collect_redundant_workers().await;
        let cleanup_count = redundant_worker_ids.len();

        for worker_id in redundant_worker_ids {
            if let Err(e) = self.unregister(&worker_id).await {
                warn!("Failed to cleanup redundant worker {}: {}", worker_id, e);
            }
        }

        let runtime_cleanup_count = self.runtime.cleanup_inactive_workers().await;

        info!(
            "Cleaned up {} redundant workers, {} inactive runtime workers",
            cleanup_count, runtime_cleanup_count
        );

        Ok(cleanup_count + runtime_cleanup_count)
    }

    pub async fn get_worker_stats(
        &self,
        worker_id: &str,
    ) -> Result<
        (
            ServiceWorker,
            crate::pwa::service_worker::runtime::ExecutionStats,
        ),
        ServiceWorkerError,
    > {
        let worker = self
            .get_worker_by_id(worker_id)
            .await
            .ok_or_else(|| ServiceWorkerError::WorkerNotFound(worker_id.to_string()))?;

        let stats = self.runtime.get_worker_stats(worker_id).await?;

        Ok((worker, stats))
    }

    pub async fn list_active_workers(&self) -> Vec<String> {
        let workers = self.workers.read().await;
        workers
            .values()
            .filter(|w| w.state == ServiceWorkerState::Activated)
            .map(|w| w.id.clone())
            .collect()
    }

    pub async fn get_worker_by_scope(&self, scope: &str) -> Option<ServiceWorker> {
        let workers = self.workers.read().await;
        workers
            .values()
            .find(|worker| worker.scope == scope)
            .cloned()
    }

    pub async fn force_update_all(&self) -> Result<Vec<String>, ServiceWorkerError> {
        let worker_ids = self.collect_all_worker_ids().await;
        let mut updated_workers = Vec::new();
        let mut errors = Vec::new();

        for worker_id in worker_ids {
            match self.update_worker(&worker_id).await {
                Ok(()) => updated_workers.push(worker_id),
                Err(e) => {
                    warn!("Failed to update worker {}: {}", worker_id, e);
                    errors.push(format!("{}: {}", worker_id, e));
                }
            }
        }

        if !errors.is_empty() {
            return Err(ServiceWorkerError::ExecutionError(format!(
                "Failed to update some workers: {}",
                errors.join(", ")
            )));
        }

        Ok(updated_workers)
    }

    async fn insert_worker(&self, worker_id: String, worker: ServiceWorker) {
        let mut workers = self.workers.write().await;
        workers.insert(worker_id, worker);
    }

    async fn remove_worker(&self, worker_id: &str) {
        let mut workers = self.workers.write().await;
        workers.remove(worker_id);
    }

    async fn update_worker_state(&self, worker_id: &str, new_state: ServiceWorkerState) {
        let mut workers = self.workers.write().await;
        if let Some(worker) = workers.get_mut(worker_id) {
            worker.state = new_state;
        }
    }

    async fn update_worker_timestamp(&self, worker_id: &str) {
        let mut workers = self.workers.write().await;
        if let Some(worker) = workers.get_mut(worker_id) {
            worker.last_update_check = SystemTime::now();
            worker.state = ServiceWorkerState::Activated;
        }
    }

    async fn cleanup_failed_worker(&self, worker_id: &str) {
        self.update_worker_state(worker_id, ServiceWorkerState::Redundant)
            .await;
        let _ = self.runtime.terminate_worker(worker_id).await;
    }

    async fn get_worker_info(&self, worker_id: &str) -> Option<(String, String)> {
        let workers = self.workers.read().await;
        workers
            .get(worker_id)
            .map(|w| (w.script_url.clone(), w.scope.clone()))
    }

    async fn get_worker_by_id(&self, worker_id: &str) -> Option<ServiceWorker> {
        let workers = self.workers.read().await;
        workers.get(worker_id).cloned()
    }

    async fn find_matching_worker(&self, url: &str) -> Option<String> {
        let workers = self.workers.read().await;
        workers
            .values()
            .find(|worker| {
                worker.state == ServiceWorkerState::Activated && url.starts_with(&worker.scope)
            })
            .map(|worker| worker.id.clone())
    }

    async fn collect_redundant_workers(&self) -> Vec<String> {
        let workers = self.workers.read().await;
        workers
            .iter()
            .filter_map(|(id, worker)| {
                if worker.state == ServiceWorkerState::Redundant {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    async fn collect_all_worker_ids(&self) -> Vec<String> {
        let workers = self.workers.read().await;
        workers.keys().cloned().collect()
    }

    fn generate_worker_id(&self, script_url: &str, scope: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        script_url.hash(&mut hasher);
        scope.hash(&mut hasher);
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .hash(&mut hasher);

        format!("sw_{:x}", hasher.finish())
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
        futures::executor::block_on(Self::new())
            .expect("Failed to create default ServiceWorkerManager")
    }
}
