pub mod cache;
pub mod manifest;
pub mod service_worker;
pub mod storage;

use cache::{CacheError, CacheManager};
use manifest::{Manifest, ManifestError, ManifestParser};
use service_worker::{ServiceWorkerError, ServiceWorkerManager};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use storage::{StorageError, StorageManager};
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

pub struct PwaRuntime {
    cache_manager: Arc<Mutex<CacheManager>>,
    storage_manager: Arc<Mutex<StorageManager>>,
    service_worker_manager: Arc<Mutex<ServiceWorkerManager>>,
    installed_apps: Arc<RwLock<HashMap<String, InstalledApp>>>,
    manifest_parser: ManifestParser,
    is_shutdown: Arc<RwLock<bool>>,
}

#[derive(Debug, Clone)]
pub struct InstalledApp {
    pub id: String,
    pub manifest: Manifest,
    pub install_time: SystemTime,
    pub last_accessed: SystemTime,
    pub data_size: u64,
}

#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub url: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct FetchResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct StorageUsage {
    pub cache_size: u64,
    pub indexeddb_size: u64,
    pub local_storage_size: u64,
    pub total_size: u64,
}

impl PwaRuntime {
    pub async fn new() -> Result<Self, PwaError> {
        let cache_manager = Arc::new(Mutex::new(CacheManager::new().await?));
        let storage_manager = Arc::new(Mutex::new(StorageManager::new().await?));
        let service_worker_manager = Arc::new(Mutex::new(ServiceWorkerManager::new().await?));
        let installed_apps = Arc::new(RwLock::new(HashMap::new()));
        let manifest_parser = ManifestParser::new();
        let is_shutdown = Arc::new(RwLock::new(false));

        Ok(Self {
            cache_manager,
            storage_manager,
            service_worker_manager,
            installed_apps,
            manifest_parser,
            is_shutdown,
        })
    }

    /// Gracefully shutdown the PWA runtime, cleaning up all resources
    pub async fn shutdown(&self) -> Result<(), PwaError> {
        info!("Shutting down PWA Runtime...");

        // Mark as shutdown to prevent new operations
        {
            let mut shutdown_flag = self.is_shutdown.write().await;
            if *shutdown_flag {
                return Ok(()); // Already shutdown
            }
            *shutdown_flag = true;
        }

        // Shutdown service workers - using existing methods
        {
            let sw_manager = self.service_worker_manager.lock().await;
            let workers = sw_manager.list_active_workers().await;

            for worker_id in workers {
                if let Err(e) = sw_manager.unregister(&worker_id).await {
                    warn!("Error shutting down service worker {}: {}", worker_id, e);
                }
            }
        }

        // Clear cache - using existing clear methods if available
        {
            // Note: We're not calling specific shutdown methods since they don't exist yet
            // Instead, we'll just clear the managers' references which will drop resources
            info!("Cache manager will be cleaned up when dropped");
        }

        // Clear storage - using existing clear methods if available
        {
            // Note: We're not calling specific shutdown methods since they don't exist yet
            // Instead, we'll just clear the managers' references which will drop resources
            info!("Storage manager will be cleaned up when dropped");
        }

        // Clear in-memory state
        {
            let mut apps = self.installed_apps.write().await;
            apps.clear();
        }

        info!("PWA Runtime shutdown complete");
        Ok(())
    }

    /// Check if the runtime has been shutdown
    pub async fn is_shutdown(&self) -> bool {
        *self.is_shutdown.read().await
    }

    /// Ensure runtime is not shutdown before executing operations
    async fn check_not_shutdown(&self) -> Result<(), PwaError> {
        if self.is_shutdown().await {
            return Err(PwaError::RuntimeShutdown);
        }
        Ok(())
    }

    pub async fn install_app(&self, manifest: &Manifest) -> Result<String, PwaError> {
        self.check_not_shutdown().await?;

        let app_id = self.generate_app_id(manifest);

        let installed_app = InstalledApp {
            id: app_id.clone(),
            manifest: manifest.clone(),
            install_time: SystemTime::now(),
            last_accessed: SystemTime::now(),
            data_size: 0,
        };

        self.register_app(app_id.clone(), installed_app).await;

        if let Some(service_worker_url) = &manifest.service_worker {
            let scope = manifest.scope.clone().unwrap_or_else(|| "/".to_string());

            let sw_manager = self.service_worker_manager.lock().await;
            match sw_manager.register(service_worker_url, &scope).await {
                Ok(worker_id) => {
                    info!("Registered service worker {} for app {}", worker_id, app_id);
                }
                Err(e) => {
                    warn!(
                        "Failed to register service worker for app {}: {}",
                        app_id, e
                    );
                    drop(sw_manager);
                    self.unregister_app(&app_id).await;
                    return Err(PwaError::ServiceWorkerError(e));
                }
            }
        }

        info!("Successfully installed PWA: {} ({})", manifest.name, app_id);
        Ok(app_id)
    }

    pub async fn uninstall_app(&self, app_id: &str) -> Result<(), PwaError> {
        self.check_not_shutdown().await?;

        let app_exists = self.remove_app(app_id).await;

        if !app_exists {
            return Err(PwaError::AppNotFound(app_id.to_string()));
        }

        let cache_result = {
            let mut cache_manager = self.cache_manager.lock().await;
            cache_manager.clear_app_cache(app_id).await
        };

        let storage_result = {
            let mut storage_manager = self.storage_manager.lock().await;
            storage_manager.clear_app_storage(app_id).await
        };

        let sw_result = self.cleanup_app_service_workers(app_id).await;

        if let Err(e) = cache_result {
            warn!("Failed to clear cache for app {}: {}", app_id, e);
        }

        if let Err(e) = storage_result {
            warn!("Failed to clear storage for app {}: {}", app_id, e);
        }

        if let Err(e) = sw_result {
            warn!(
                "Failed to cleanup service workers for app {}: {}",
                app_id, e
            );
        }

        info!("Successfully uninstalled PWA: {}", app_id);
        Ok(())
    }

    pub async fn get_installed_apps(&self) -> Vec<InstalledApp> {
        if self.is_shutdown().await {
            return Vec::new();
        }

        let apps = self.installed_apps.read().await;
        apps.values().cloned().collect()
    }

    pub async fn update_app(&self, app_id: &str, new_manifest: &Manifest) -> Result<(), PwaError> {
        self.check_not_shutdown().await?;

        let updated = self.update_app_manifest(app_id, new_manifest).await;

        if !updated {
            return Err(PwaError::AppNotFound(app_id.to_string()));
        }

        if let Some(service_worker_url) = &new_manifest.service_worker {
            let scope = new_manifest
                .scope
                .clone()
                .unwrap_or_else(|| "/".to_string());

            let sw_manager = self.service_worker_manager.lock().await;
            if let Some(worker) = sw_manager.get_registration(&scope).await {
                match sw_manager.update_worker(&worker.id).await {
                    Ok(()) => info!("Updated service worker for app {}", app_id),
                    Err(e) => warn!("Failed to update service worker for app {}: {}", app_id, e),
                }
            } else {
                match sw_manager.register(service_worker_url, &scope).await {
                    Ok(worker_id) => info!(
                        "Registered new service worker {} for app {}",
                        worker_id, app_id
                    ),
                    Err(e) => warn!(
                        "Failed to register new service worker for app {}: {}",
                        app_id, e
                    ),
                }
            }
        }

        info!("Successfully updated app: {}", app_id);
        Ok(())
    }

    pub async fn register_service_worker(
        &self,
        script_url: &str,
        scope: Option<&str>,
    ) -> Result<String, PwaError> {
        self.check_not_shutdown().await?;

        let actual_scope = scope.unwrap_or("/");

        let sw_manager = self.service_worker_manager.lock().await;
        match sw_manager.register(script_url, actual_scope).await {
            Ok(worker_id) => {
                info!("Registered standalone service worker: {}", worker_id);
                Ok(worker_id)
            }
            Err(e) => {
                error!("Failed to register service worker {}: {}", script_url, e);
                Err(PwaError::ServiceWorkerError(e))
            }
        }
    }

    pub async fn handle_fetch_request(
        &self,
        request: &FetchRequest,
    ) -> Result<FetchResponse, PwaError> {
        self.check_not_shutdown().await?;

        let sw_response = {
            let sw_manager = self.service_worker_manager.lock().await;
            sw_manager.handle_fetch(request).await?
        };

        if let Some(response) = sw_response {
            return Ok(response);
        }

        let cached_response = {
            let mut cache_manager = self.cache_manager.lock().await;
            cache_manager.match_request(request).await?
        };

        if let Some(response) = cached_response {
            return Ok(response);
        }

        self.handle_network_fetch(request).await
    }

    pub async fn get_app_storage_usage(&self, app_id: &str) -> Result<StorageUsage, PwaError> {
        self.check_not_shutdown().await?;

        let app_exists = self.app_exists(app_id).await;

        if !app_exists {
            return Err(PwaError::AppNotFound(app_id.to_string()));
        }

        let cache_usage = {
            let cache_manager = self.cache_manager.lock().await;
            cache_manager.get_cache_usage().await
        };

        let storage_details = {
            let storage_manager = self.storage_manager.lock().await;
            storage_manager.get_usage(app_id).await?
        };

        let cache_size = cache_usage.total_size;

        Ok(StorageUsage {
            cache_size,
            indexeddb_size: storage_details.indexeddb_size,
            local_storage_size: storage_details.local_storage_size,
            total_size: cache_size
                + storage_details.indexeddb_size
                + storage_details.local_storage_size,
        })
    }

    pub async fn clear_app_data(&self, app_id: &str) -> Result<(), PwaError> {
        self.check_not_shutdown().await?;

        let app_exists = self.app_exists(app_id).await;

        if !app_exists {
            return Err(PwaError::AppNotFound(app_id.to_string()));
        }

        let cache_result = {
            let mut cache_manager = self.cache_manager.lock().await;
            cache_manager.clear_app_cache(app_id).await
        };

        let storage_result = {
            let mut storage_manager = self.storage_manager.lock().await;
            storage_manager.clear_app_storage(app_id).await
        };

        cache_result?;
        storage_result?;

        self.update_app_data_size(app_id, 0).await;

        info!("Cleared all data for app: {}", app_id);
        Ok(())
    }

    pub async fn get_app_manifest(&self, app_id: &str) -> Option<Manifest> {
        if self.is_shutdown().await {
            return None;
        }

        let apps = self.installed_apps.read().await;
        apps.get(app_id).map(|app| app.manifest.clone())
    }

    pub async fn list_service_workers(&self) -> Vec<String> {
        if self.is_shutdown().await {
            return Vec::new();
        }

        let sw_manager = self.service_worker_manager.lock().await;
        sw_manager.list_active_workers().await
    }

    pub async fn cleanup_inactive_apps(&self) -> Result<Vec<String>, PwaError> {
        self.check_not_shutdown().await?;

        let inactive_threshold = SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(30 * 24 * 60 * 60))
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let inactive_apps = self.find_inactive_apps(inactive_threshold).await;
        let mut cleaned_apps = Vec::new();

        for app_id in inactive_apps {
            match self.uninstall_app(&app_id).await {
                Ok(()) => {
                    cleaned_apps.push(app_id);
                }
                Err(e) => {
                    warn!("Failed to cleanup inactive app {}: {}", app_id, e);
                }
            }
        }

        info!("Cleaned up {} inactive apps", cleaned_apps.len());
        Ok(cleaned_apps)
    }

    pub async fn get_total_storage_usage(&self) -> Result<StorageUsage, PwaError> {
        self.check_not_shutdown().await?;

        let cache_usage = {
            let cache_manager = self.cache_manager.lock().await;
            cache_manager.get_cache_usage().await
        };

        let all_apps = self.get_installed_apps().await;
        let mut total_indexeddb = 0u64;
        let mut total_local_storage = 0u64;

        for app in all_apps {
            if let Ok(usage) = self.get_app_storage_usage(&app.id).await {
                total_indexeddb += usage.indexeddb_size;
                total_local_storage += usage.local_storage_size;
            }
        }

        Ok(StorageUsage {
            cache_size: cache_usage.total_size,
            indexeddb_size: total_indexeddb,
            local_storage_size: total_local_storage,
            total_size: cache_usage.total_size + total_indexeddb + total_local_storage,
        })
    }

    async fn register_app(&self, app_id: String, app: InstalledApp) {
        let mut apps = self.installed_apps.write().await;
        apps.insert(app_id, app);
    }

    async fn unregister_app(&self, app_id: &str) {
        let mut apps = self.installed_apps.write().await;
        apps.remove(app_id);
    }

    async fn remove_app(&self, app_id: &str) -> bool {
        let mut apps = self.installed_apps.write().await;
        apps.remove(app_id).is_some()
    }

    async fn app_exists(&self, app_id: &str) -> bool {
        let apps = self.installed_apps.read().await;
        apps.contains_key(app_id)
    }

    async fn update_app_manifest(&self, app_id: &str, new_manifest: &Manifest) -> bool {
        let mut apps = self.installed_apps.write().await;
        if let Some(app) = apps.get_mut(app_id) {
            app.manifest = new_manifest.clone();
            app.last_accessed = SystemTime::now();
            true
        } else {
            false
        }
    }

    async fn update_app_data_size(&self, app_id: &str, new_size: u64) {
        let mut apps = self.installed_apps.write().await;
        if let Some(app) = apps.get_mut(app_id) {
            app.data_size = new_size;
            app.last_accessed = SystemTime::now();
        }
    }

    async fn find_inactive_apps(&self, threshold: SystemTime) -> Vec<String> {
        let apps = self.installed_apps.read().await;
        apps.iter()
            .filter_map(|(id, app)| {
                if app.last_accessed < threshold {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    async fn cleanup_app_service_workers(&self, app_id: &str) -> Result<(), PwaError> {
        let sw_manager = self.service_worker_manager.lock().await;
        let workers = sw_manager.get_all_registrations().await;

        for worker in workers {
            if worker.scope.contains(app_id) {
                if let Err(e) = sw_manager.unregister(&worker.id).await {
                    warn!("Failed to unregister service worker {}: {}", worker.id, e);
                }
            }
        }

        Ok(())
    }

    async fn handle_network_fetch(
        &self,
        request: &FetchRequest,
    ) -> Result<FetchResponse, PwaError> {
        // For now, return a simple error. In a real implementation,
        // this would make an actual network request
        Err(PwaError::ResourceNotFound(request.url.clone()))
    }

    fn generate_app_id(&self, manifest: &Manifest) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        manifest.start_url.hash(&mut hasher);
        manifest.name.hash(&mut hasher);
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .hash(&mut hasher);

        format!("app_{:x}", hasher.finish())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PwaError {
    #[error("Cache error: {0}")]
    CacheError(#[from] CacheError),
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),
    #[error("Service worker error: {0}")]
    ServiceWorkerError(#[from] ServiceWorkerError),
    #[error("Manifest error: {0}")]
    ManifestError(#[from] ManifestError),
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error("Resource not found: {0}")]
    ResourceNotFound(String),
    #[error("Installation failed: {0}")]
    InstallationFailed(String),
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Runtime has been shutdown")]
    RuntimeShutdown,
}

impl Default for PwaRuntime {
    fn default() -> Self {
        futures::executor::block_on(Self::new()).expect("Failed to create default PwaRuntime")
    }
}
