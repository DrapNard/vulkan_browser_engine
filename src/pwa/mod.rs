pub mod cache;
pub mod manifest;
pub mod service_worker;
pub mod storage;

use cache::CacheManager;
use manifest::{Manifest, ManifestParser};
use service_worker::ServiceWorkerRuntime;
use storage::StorageManager;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct PwaRuntime {
    cache_manager: Arc<RwLock<CacheManager>>,
    storage_manager: Arc<RwLock<StorageManager>>,
    service_worker_runtime: Arc<RwLock<ServiceWorkerRuntime>>,
    installed_apps: Arc<RwLock<HashMap<String, InstalledApp>>>,
    manifest_parser: ManifestParser,
}

#[derive(Debug, Clone)]
pub struct InstalledApp {
    pub id: String,
    pub manifest: Manifest,
    pub install_time: std::time::SystemTime,
    pub last_accessed: std::time::SystemTime,
    pub data_size: u64,
}

impl PwaRuntime {
    pub async fn new() -> Result<Self, PwaError> {
        let cache_manager = Arc::new(RwLock::new(CacheManager::new().await?));
        let storage_manager = Arc::new(RwLock::new(StorageManager::new().await?));
        let service_worker_runtime = Arc::new(RwLock::new(ServiceWorkerRuntime::new()));
        let installed_apps = Arc::new(RwLock::new(HashMap::new()));
        let manifest_parser = ManifestParser::new();

        Ok(Self {
            cache_manager,
            storage_manager,
            service_worker_runtime,
            installed_apps,
            manifest_parser,
        })
    }

    pub async fn install_app(&self, manifest: &Manifest) -> Result<String, PwaError> {
        let app_id = self.generate_app_id(manifest);
        
        let installed_app = InstalledApp {
            id: app_id.clone(),
            manifest: manifest.clone(),
            install_time: std::time::SystemTime::now(),
            last_accessed: std::time::SystemTime::now(),
            data_size: 0,
        };

        {
            let mut apps = self.installed_apps.write().await;
            apps.insert(app_id.clone(), installed_app);
        }

        if let Some(service_worker_url) = &manifest.service_worker {
            let sw_runtime = self.service_worker_runtime.clone();
            let mut runtime = sw_runtime.write().await;
            runtime.register(service_worker_url, &manifest.scope.clone().unwrap_or("/".to_string())).await?;
        }

        log::info!("Installed PWA: {} ({})", manifest.name, app_id);
        Ok(app_id)
    }

    pub async fn uninstall_app(&self, app_id: &str) -> Result<(), PwaError> {
        let mut apps = self.installed_apps.write().await;
        if let Some(app) = apps.remove(app_id) {
            let mut cache_manager = self.cache_manager.write().await;
            cache_manager.clear_app_cache(app_id).await?;
            
            let mut storage_manager = self.storage_manager.write().await;
            storage_manager.clear_app_storage(app_id).await?;
            
            log::info!("Uninstalled PWA: {}", app_id);
            Ok(())
        } else {
            Err(PwaError::AppNotFound(app_id.to_string()))
        }
    }

    pub async fn get_installed_apps(&self) -> Vec<InstalledApp> {
        let apps = self.installed_apps.read().await;
        apps.values().cloned().collect()
    }

    pub async fn update_app(&self, app_id: &str, new_manifest: &Manifest) -> Result<(), PwaError> {
        let mut apps = self.installed_apps.write().await;
        if let Some(app) = apps.get_mut(app_id) {
            app.manifest = new_manifest.clone();
            app.last_accessed = std::time::SystemTime::now();
            Ok(())
        } else {
            Err(PwaError::AppNotFound(app_id.to_string()))
        }
    }

    pub async fn register_service_worker(&self, script_url: &str) -> Result<String, PwaError> {
        let mut runtime = self.service_worker_runtime.write().await;
        runtime.register(script_url, "/").await
    }

    pub async fn handle_fetch_request(&self, request: &FetchRequest) -> Result<FetchResponse, PwaError> {
        let sw_runtime = self.service_worker_runtime.read().await;
        if let Some(response) = sw_runtime.handle_fetch(request).await? {
            return Ok(response);
        }

        let cache_manager = self.cache_manager.read().await;
        if let Some(cached_response) = cache_manager.match_request(request).await? {
            return Ok(cached_response);
        }

        Err(PwaError::ResourceNotFound(request.url.clone()))
    }

    fn generate_app_id(&self, manifest: &Manifest) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        manifest.start_url.hash(&mut hasher);
        manifest.name.hash(&mut hasher);
        format!("app_{:x}", hasher.finish())
    }

    pub async fn get_app_storage_usage(&self, app_id: &str) -> Result<StorageUsage, PwaError> {
        let storage_manager = self.storage_manager.read().await;
        storage_manager.get_usage(app_id).await
    }

    pub async fn clear_app_data(&self, app_id: &str) -> Result<(), PwaError> {
        let mut cache_manager = self.cache_manager.write().await;
        cache_manager.clear_app_cache(app_id).await?;
        
        let mut storage_manager = self.storage_manager.write().await;
        storage_manager.clear_app_storage(app_id).await?;
        
        Ok(())
    }
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

#[derive(Debug, thiserror::Error)]
pub enum PwaError {
    #[error("Cache error: {0}")]
    CacheError(#[from] cache::CacheError),
    #[error("Storage error: {0}")]
    StorageError(#[from] storage::StorageError),
    #[error("Service worker error: {0}")]
    ServiceWorkerError(#[from] service_worker::ServiceWorkerError),
    #[error("Manifest error: {0}")]
    ManifestError(#[from] manifest::ManifestError),
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error("Resource not found: {0}")]
    ResourceNotFound(String),
    #[error("Installation failed: {0}")]
    InstallationFailed(String),
}

impl Default for PwaRuntime {
    fn default() -> Self {
        futures::executor::block_on(async { Self::new().await.unwrap() })
    }
}