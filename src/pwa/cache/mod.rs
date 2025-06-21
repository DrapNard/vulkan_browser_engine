pub mod strategy;

pub use strategy::*;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tokio::fs;
use serde::{Deserialize, Serialize};

pub struct CacheManager {
    cache_root: PathBuf,
    caches: HashMap<String, Cache>,
    global_quota: u64,
    used_space: u64,
}

pub struct Cache {
    name: String,
    entries: HashMap<String, CacheEntry>,
    strategy: CacheStrategy,
    max_size: u64,
    current_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub url: String,
    pub response: CachedResponse,
    pub stored_at: SystemTime,
    pub expires_at: Option<SystemTime>,
    pub size: u64,
    pub access_count: u32,
    pub last_accessed: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl CacheManager {
    pub async fn new() -> Result<Self, CacheError> {
        let cache_root = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("./cache"))
            .join("vulkan-renderer");
        
        fs::create_dir_all(&cache_root).await
            .map_err(|e| CacheError::IoError(e.to_string()))?;

        Ok(Self {
            cache_root,
            caches: HashMap::new(),
            global_quota: 50 * 1024 * 1024 * 1024, // 50GB
            used_space: 0,
        })
    }

    pub async fn open_cache(&mut self, name: &str) -> Result<&mut Cache, CacheError> {
        if !self.caches.contains_key(name) {
            let cache = Cache::new(name.to_string(), CacheStrategy::CacheFirst).await?;
            self.caches.insert(name.to_string(), cache);
        }
        Ok(self.caches.get_mut(name).unwrap())
    }

    pub async fn delete_cache(&mut self, name: &str) -> Result<bool, CacheError> {
        if let Some(cache) = self.caches.remove(name) {
            self.used_space -= cache.current_size;
            
            let cache_dir = self.cache_root.join(name);
            if cache_dir.exists() {
                fs::remove_dir_all(cache_dir).await
                    .map_err(|e| CacheError::IoError(e.to_string()))?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn match_request(&self, request: &crate::pwa::FetchRequest) -> Result<Option<crate::pwa::FetchResponse>, CacheError> {
        for cache in self.caches.values() {
            if let Some(entry) = cache.match_url(&request.url).await {
                if !self.is_expired(&entry) {
                    return Ok(Some(crate::pwa::FetchResponse {
                        status: entry.response.status,
                        headers: entry.response.headers.clone(),
                        body: entry.response.body.clone(),
                    }));
                }
            }
        }
        Ok(None)
    }

    pub async fn add_to_cache(&mut self, cache_name: &str, url: &str, response: &crate::pwa::FetchResponse) -> Result<(), CacheError> {
        let cache = self.open_cache(cache_name).await?;
        
        let entry = CacheEntry {
            url: url.to_string(),
            response: CachedResponse {
                status: response.status,
                headers: response.headers.clone(),
                body: response.body.clone(),
            },
            stored_at: SystemTime::now(),
            expires_at: self.calculate_expiry(&response.headers),
            size: response.body.len() as u64,
            access_count: 0,
            last_accessed: SystemTime::now(),
        };

        if self.used_space + entry.size > self.global_quota {
            self.evict_entries().await?;
        }

        cache.put(url, entry).await?;
        self.used_space += response.body.len() as u64;
        
        Ok(())
    }

    pub async fn clear_app_cache(&mut self, app_id: &str) -> Result<(), CacheError> {
        let caches_to_remove: Vec<String> = self.caches.keys()
            .filter(|name| name.starts_with(&format!("app_{}_", app_id)))
            .cloned()
            .collect();
        
        for cache_name in caches_to_remove {
            self.delete_cache(&cache_name).await?;
        }
        
        Ok(())
    }

    async fn evict_entries(&mut self) -> Result<(), CacheError> {
        let mut entries_to_remove = Vec::new();
        
        for (cache_name, cache) in &self.caches {
            for (url, entry) in &cache.entries {
                if self.is_expired(entry) || entry.access_count == 0 {
                    entries_to_remove.push((cache_name.clone(), url.clone()));
                }
            }
        }
        
        entries_to_remove.sort_by_key(|(_, _)| {
            // Sort by access count and last accessed time
            0
        });
        
        for (cache_name, url) in entries_to_remove.iter().take(100) {
            if let Some(cache) = self.caches.get_mut(cache_name) {
                if let Some(entry) = cache.entries.remove(url) {
                    self.used_space -= entry.size;
                }
            }
        }
        
        Ok(())
    }

    fn is_expired(&self, entry: &CacheEntry) -> bool {
        if let Some(expires_at) = entry.expires_at {
            SystemTime::now() > expires_at
        } else {
            false
        }
    }

    fn calculate_expiry(&self, headers: &HashMap<String, String>) -> Option<SystemTime> {
        if let Some(cache_control) = headers.get("cache-control") {
            if let Some(max_age_start) = cache_control.find("max-age=") {
                let max_age_str = &cache_control[max_age_start + 8..];
                if let Some(max_age_end) = max_age_str.find(',') {
                    let max_age_str = &max_age_str[..max_age_end];
                    if let Ok(max_age) = max_age_str.trim().parse::<u64>() {
                        return Some(SystemTime::now() + Duration::from_secs(max_age));
                    }
                } else if let Ok(max_age) = max_age_str.trim().parse::<u64>() {
                    return Some(SystemTime::now() + Duration::from_secs(max_age));
                }
            }
        }
        
        if let Some(_expires) = headers.get("expires") {
            // Parse HTTP date format
            // For simplicity, returning None here
        }
        
        None
    }

    pub async fn get_cache_usage(&self) -> CacheUsage {
        CacheUsage {
            total_size: self.used_space,
            cache_count: self.caches.len(),
            quota: self.global_quota,
            usage_percentage: (self.used_space as f64 / self.global_quota as f64) * 100.0,
        }
    }
}

impl Cache {
    async fn new(name: String, strategy: CacheStrategy) -> Result<Self, CacheError> {
        Ok(Self {
            name,
            entries: HashMap::new(),
            strategy,
            max_size: 100 * 1024 * 1024, // 100MB per cache
            current_size: 0,
        })
    }

    async fn match_url(&self, url: &str) -> Option<&CacheEntry> {
        self.entries.get(url)
    }

    async fn put(&mut self, url: &str, entry: CacheEntry) -> Result<(), CacheError> {
        if self.current_size + entry.size > self.max_size {
            self.evict_lru().await?;
        }
        
        self.current_size += entry.size;
        self.entries.insert(url.to_string(), entry);
        Ok(())
    }

    async fn evict_lru(&mut self) -> Result<(), CacheError> {
        let mut oldest_entry: Option<(String, SystemTime)> = None;
        
        for (url, entry) in &self.entries {
            if oldest_entry.is_none() || entry.last_accessed < oldest_entry.as_ref().unwrap().1 {
                oldest_entry = Some((url.clone(), entry.last_accessed));
            }
        }
        
        if let Some((url, _)) = oldest_entry {
            if let Some(entry) = self.entries.remove(&url) {
                self.current_size -= entry.size;
            }
        }
        
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CacheUsage {
    pub total_size: u64,
    pub cache_count: usize,
    pub quota: u64,
    pub usage_percentage: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("IO error: {0}")]
    IoError(String),
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("Cache not found: {0}")]
    CacheNotFound(String),
    #[error("Quota exceeded")]
    QuotaExceeded,
    #[error("Invalid cache entry")]
    InvalidEntry,
}

impl Default for CacheManager {
    fn default() -> Self {
        futures::executor::block_on(async { Self::new().await.unwrap() })
    }
}
