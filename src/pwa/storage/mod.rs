pub mod quota;

pub use quota::*;

use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs;
use serde::{Deserialize, Serialize};

pub struct StorageManager {
    storage_root: PathBuf,
    databases: HashMap<String, IndexedDatabase>,
    local_storage: HashMap<String, HashMap<String, String>>,
    session_storage: HashMap<String, HashMap<String, String>>,
    quota_manager: QuotaManager,
}

pub struct IndexedDatabase {
    name: String,
    version: u32,
    object_stores: HashMap<String, ObjectStore>,
}

pub struct ObjectStore {
    name: String,
    key_path: Option<String>,
    auto_increment: bool,
    data: HashMap<String, serde_json::Value>,
    indexes: HashMap<String, Index>,
}

pub struct Index {
    name: String,
    key_path: String,
    unique: bool,
    multientry: bool,
}

impl StorageManager {
    pub async fn new() -> Result<Self, StorageError> {
        let storage_root = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("./data"))
            .join("vulkan-renderer")
            .join("storage");
        
        fs::create_dir_all(&storage_root).await
            .map_err(|e| StorageError::IoError(e.to_string()))?;

        Ok(Self {
            storage_root,
            databases: HashMap::new(),
            local_storage: HashMap::new(),
            session_storage: HashMap::new(),
            quota_manager: QuotaManager::new(),
        })
    }

    pub async fn open_database(&mut self, name: &str, version: u32) -> Result<&mut IndexedDatabase, StorageError> {
        let key = format!("{}:{}", name, version);
        
        if !self.databases.contains_key(&key) {
            let db = IndexedDatabase {
                name: name.to_string(),
                version,
                object_stores: HashMap::new(),
            };
            self.databases.insert(key.clone(), db);
        }

        Ok(self.databases.get_mut(&key).unwrap())
    }

    pub async fn delete_database(&mut self, name: &str) -> Result<(), StorageError> {
        let keys_to_remove: Vec<String> = self.databases.keys()
            .filter(|key| key.starts_with(&format!("{}:", name)))
            .cloned()
            .collect();

        for key in keys_to_remove {
            self.databases.remove(&key);
        }

        let db_path = self.storage_root.join(format!("{}.db", name));
        if db_path.exists() {
            fs::remove_file(db_path).await
                .map_err(|e| StorageError::IoError(e.to_string()))?;
        }

        Ok(())
    }

    pub async fn create_object_store(&mut self, db_name: &str, db_version: u32, store_name: &str, key_path: Option<String>, auto_increment: bool) -> Result<(), StorageError> {
        let db = self.open_database(db_name, db_version).await?;
        
        let store = ObjectStore {
            name: store_name.to_string(),
            key_path,
            auto_increment,
            data: HashMap::new(),
            indexes: HashMap::new(),
        };

        db.object_stores.insert(store_name.to_string(), store);
        Ok(())
    }

    pub async fn put_object(&mut self, db_name: &str, db_version: u32, store_name: &str, key: &str, value: serde_json::Value) -> Result<(), StorageError> {
        let db = self.open_database(db_name, db_version).await?;
        
        if let Some(store) = db.object_stores.get_mut(store_name) {
            store.data.insert(key.to_string(), value);
            Ok(())
        } else {
            Err(StorageError::ObjectStoreNotFound(store_name.to_string()))
        }
    }

    pub async fn get_object(&self, db_name: &str, db_version: u32, store_name: &str, key: &str) -> Result<Option<serde_json::Value>, StorageError> {
        let db_key = format!("{}:{}", db_name, db_version);
        
        if let Some(db) = self.databases.get(&db_key) {
            if let Some(store) = db.object_stores.get(store_name) {
                Ok(store.data.get(key).cloned())
            } else {
                Err(StorageError::ObjectStoreNotFound(store_name.to_string()))
            }
        } else {
            Err(StorageError::DatabaseNotFound(db_name.to_string()))
        }
    }

    pub async fn delete_object(&mut self, db_name: &str, db_version: u32, store_name: &str, key: &str) -> Result<bool, StorageError> {
        let db = self.open_database(db_name, db_version).await?;
        
        if let Some(store) = db.object_stores.get_mut(store_name) {
            Ok(store.data.remove(key).is_some())
        } else {
            Err(StorageError::ObjectStoreNotFound(store_name.to_string()))
        }
    }

    pub async fn set_local_storage(&mut self, origin: &str, key: &str, value: &str) -> Result<(), StorageError> {
        let storage = self.local_storage.entry(origin.to_string()).or_insert_with(HashMap::new);
        storage.insert(key.to_string(), value.to_string());
        self.persist_local_storage(origin).await?;
        Ok(())
    }

    pub async fn get_local_storage(&self, origin: &str, key: &str) -> Option<String> {
        self.local_storage.get(origin)?.get(key).cloned()
    }

    pub async fn remove_local_storage(&mut self, origin: &str, key: &str) -> Result<(), StorageError> {
        if let Some(storage) = self.local_storage.get_mut(origin) {
            storage.remove(key);
            self.persist_local_storage(origin).await?;
        }
        Ok(())
    }

    pub async fn clear_local_storage(&mut self, origin: &str) -> Result<(), StorageError> {
        self.local_storage.remove(origin);
        let storage_file = self.storage_root.join(format!("{}_localStorage.json", origin));
        if storage_file.exists() {
            fs::remove_file(storage_file).await
                .map_err(|e| StorageError::IoError(e.to_string()))?;
        }
        Ok(())
    }

    pub async fn set_session_storage(&mut self, origin: &str, key: &str, value: &str) -> Result<(), StorageError> {
        let storage = self.session_storage.entry(origin.to_string()).or_insert_with(HashMap::new);
        storage.insert(key.to_string(), value.to_string());
        Ok(())
    }

    pub async fn get_session_storage(&self, origin: &str, key: &str) -> Option<String> {
        self.session_storage.get(origin)?.get(key).cloned()
    }

    pub async fn clear_app_storage(&mut self, app_id: &str) -> Result<(), StorageError> {
        let dbs_to_remove: Vec<String> = self.databases.keys()
            .filter(|key| key.contains(&format!("app_{}", app_id)))
            .cloned()
            .collect();

        for db_key in dbs_to_remove {
            self.databases.remove(&db_key);
        }

        self.local_storage.remove(&format!("app_{}", app_id));
        self.session_storage.remove(&format!("app_{}", app_id));

        Ok(())
    }

    pub async fn get_usage(&self, app_id: &str) -> Result<crate::pwa::StorageUsage, StorageError> {
        let mut indexeddb_size = 0u64;
        let mut local_storage_size = 0u64;

        for (db_key, db) in &self.databases {
            if db_key.contains(&format!("app_{}", app_id)) {
                for store in db.object_stores.values() {
                    for value in store.data.values() {
                        indexeddb_size += serde_json::to_string(value).unwrap().len() as u64;
                    }
                }
            }
        }

        if let Some(local_data) = self.local_storage.get(&format!("app_{}", app_id)) {
            for (key, value) in local_data {
                local_storage_size += (key.len() + value.len()) as u64;
            }
        }

        Ok(crate::pwa::StorageUsage {
            cache_size: 0, // This would be calculated by the cache manager
            indexeddb_size,
            local_storage_size,
            total_size: indexeddb_size + local_storage_size,
        })
    }

    async fn persist_local_storage(&self, origin: &str) -> Result<(), StorageError> {
        if let Some(data) = self.local_storage.get(origin) {
            let storage_file = self.storage_root.join(format!("{}_localStorage.json", origin));
            let json_data = serde_json::to_string_pretty(data)
                .map_err(|e| StorageError::SerializationError(e.to_string()))?;
            
            fs::write(storage_file, json_data).await
                .map_err(|e| StorageError::IoError(e.to_string()))?;
        }
        Ok(())
    }

    pub async fn load_persistent_data(&mut self, origin: &str) -> Result<(), StorageError> {
        let storage_file = self.storage_root.join(format!("{}_localStorage.json", origin));
        
        if storage_file.exists() {
            let content = fs::read_to_string(storage_file).await
                .map_err(|e| StorageError::IoError(e.to_string()))?;
            
            let data: HashMap<String, String> = serde_json::from_str(&content)
                .map_err(|e| StorageError::SerializationError(e.to_string()))?;
            
            self.local_storage.insert(origin.to_string(), data);
        }
        
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    IoError(String),
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("Database not found: {0}")]
    DatabaseNotFound(String),
    #[error("Object store not found: {0}")]
    ObjectStoreNotFound(String),
    #[error("Quota exceeded")]
    QuotaExceeded,
    #[error("Permission denied")]
    PermissionDenied,
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
}

impl Default for StorageManager {
    fn default() -> Self {
        futures::executor::block_on(async { Self::new().await.unwrap() })
    }
}