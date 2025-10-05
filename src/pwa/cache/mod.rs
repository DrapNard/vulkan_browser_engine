#![allow(dead_code)]

pub mod strategy;

pub use strategy::*;

use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::fs;

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
    access_order: BinaryHeap<Reverse<AccessOrder>>,
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AccessOrder {
    last_accessed: SystemTime,
    url: String,
}

impl CacheManager {
    pub async fn new() -> Result<Self, CacheError> {
        let cache_root = Self::get_cache_directory().join("vulkan-renderer");

        fs::create_dir_all(&cache_root)
            .await
            .map_err(|e| CacheError::IoError(e.to_string()))?;

        Ok(Self {
            cache_root,
            caches: HashMap::new(),
            global_quota: 50 * 1024 * 1024 * 1024,
            used_space: 0,
        })
    }

    fn get_cache_directory() -> PathBuf {
        std::env::var("CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                if cfg!(target_os = "windows") {
                    std::env::var("LOCALAPPDATA")
                        .map(PathBuf::from)
                        .unwrap_or_else(|_| PathBuf::from("./cache"))
                } else if cfg!(target_os = "macos") {
                    std::env::var("HOME")
                        .map(|home| PathBuf::from(home).join("Library/Caches"))
                        .unwrap_or_else(|_| PathBuf::from("./cache"))
                } else {
                    std::env::var("XDG_CACHE_HOME")
                        .map(PathBuf::from)
                        .or_else(|_| {
                            std::env::var("HOME").map(|home| PathBuf::from(home).join(".cache"))
                        })
                        .unwrap_or_else(|_| PathBuf::from("./cache"))
                }
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
            self.used_space = self.used_space.saturating_sub(cache.current_size);

            let cache_dir = self.cache_root.join(name);
            if cache_dir.exists() {
                fs::remove_dir_all(cache_dir)
                    .await
                    .map_err(|e| CacheError::IoError(e.to_string()))?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn match_request(
        &mut self,
        request: &crate::pwa::FetchRequest,
    ) -> Result<Option<crate::pwa::FetchResponse>, CacheError> {
        for cache in self.caches.values_mut() {
            if let Some(entry) = cache.match_url_mut(&request.url).await {
                if !Self::is_expired(entry) {
                    entry.access_count = entry.access_count.saturating_add(1);
                    entry.last_accessed = SystemTime::now();

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

    pub async fn add_to_cache(
        &mut self,
        cache_name: &str,
        url: &str,
        response: &crate::pwa::FetchResponse,
    ) -> Result<(), CacheError> {
        let entry_size = response.body.len() as u64;

        if self.used_space + entry_size > self.global_quota {
            self.evict_global_entries().await?;
        }

        let entry = CacheEntry {
            url: url.to_string(),
            response: CachedResponse {
                status: response.status,
                headers: response.headers.clone(),
                body: response.body.clone(),
            },
            stored_at: SystemTime::now(),
            expires_at: Self::calculate_expiry(&response.headers),
            size: entry_size,
            access_count: 1,
            last_accessed: SystemTime::now(),
        };

        let cache = self.open_cache(cache_name).await?;
        cache.put(url, entry).await?;
        self.used_space += entry_size;

        Ok(())
    }

    pub async fn clear_app_cache(&mut self, app_id: &str) -> Result<(), CacheError> {
        let cache_prefix = format!("app_{}_", app_id);
        let caches_to_remove: Vec<String> = self
            .caches
            .keys()
            .filter(|name| name.starts_with(&cache_prefix))
            .cloned()
            .collect();

        for cache_name in caches_to_remove {
            self.delete_cache(&cache_name).await?;
        }

        Ok(())
    }

    async fn evict_global_entries(&mut self) -> Result<(), CacheError> {
        let mut eviction_candidates = Vec::new();

        for (cache_name, cache) in &self.caches {
            for (url, entry) in &cache.entries {
                if Self::is_expired(entry) {
                    eviction_candidates.push((cache_name.clone(), url.clone(), 0u64, entry.size));
                } else {
                    let priority = Self::calculate_eviction_priority(entry);
                    eviction_candidates.push((
                        cache_name.clone(),
                        url.clone(),
                        priority,
                        entry.size,
                    ));
                }
            }
        }

        eviction_candidates.sort_by_key(|&(_, _, priority, _)| priority);

        let mut freed_space = 0u64;
        let target_space = self.global_quota / 10;

        for (cache_name, url, _, size) in eviction_candidates {
            if freed_space >= target_space {
                break;
            }

            if let Some(cache) = self.caches.get_mut(&cache_name) {
                if cache.entries.remove(&url).is_some() {
                    cache.current_size = cache.current_size.saturating_sub(size);
                    self.used_space = self.used_space.saturating_sub(size);
                    freed_space += size;
                }
            }
        }

        Ok(())
    }

    fn calculate_eviction_priority(entry: &CacheEntry) -> u64 {
        let age_weight = entry
            .stored_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let access_weight = entry.access_count as u64 * 1000;
        let recency_weight = entry
            .last_accessed
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        age_weight
            .saturating_sub(access_weight)
            .saturating_sub(recency_weight / 10)
    }

    fn is_expired(entry: &CacheEntry) -> bool {
        entry
            .expires_at
            .map(|expires_at| SystemTime::now() > expires_at)
            .unwrap_or(false)
    }

    fn calculate_expiry(headers: &HashMap<String, String>) -> Option<SystemTime> {
        if let Some(cache_control) = headers.get("cache-control") {
            if let Some(max_age) = Self::extract_max_age(cache_control) {
                return Some(SystemTime::now() + Duration::from_secs(max_age));
            }
        }

        if let Some(expires_header) = headers.get("expires") {
            if let Some(expires_time) = Self::parse_http_date(expires_header) {
                return Some(expires_time);
            }
        }

        None
    }

    fn extract_max_age(cache_control: &str) -> Option<u64> {
        cache_control.split(',').find_map(|directive| {
            directive
                .trim()
                .strip_prefix("max-age=")
                .map(str::trim)
                .and_then(|value| value.parse().ok())
        })
    }

    fn parse_http_date(date_str: &str) -> Option<SystemTime> {
        let formats = [
            "%a, %d %b %Y %H:%M:%S GMT",
            "%A, %d-%b-%y %H:%M:%S GMT",
            "%a %b %d %H:%M:%S %Y",
        ];

        for format in &formats {
            if let Ok(datetime) = chrono::DateTime::parse_from_str(date_str, format) {
                return Some(UNIX_EPOCH + Duration::from_secs(datetime.timestamp() as u64));
            }
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

    pub async fn cleanup_expired(&mut self) -> Result<u64, CacheError> {
        let mut cleaned_size = 0u64;

        for cache in self.caches.values_mut() {
            let expired_urls: Vec<String> = cache
                .entries
                .iter()
                .filter(|(_, entry)| Self::is_expired(entry))
                .map(|(url, _)| url.clone())
                .collect();

            for url in expired_urls {
                if let Some(entry) = cache.entries.remove(&url) {
                    cache.current_size = cache.current_size.saturating_sub(entry.size);
                    cleaned_size += entry.size;
                }
            }
        }

        self.used_space = self.used_space.saturating_sub(cleaned_size);
        Ok(cleaned_size)
    }
}

impl Cache {
    async fn new(name: String, strategy: CacheStrategy) -> Result<Self, CacheError> {
        Ok(Self {
            name,
            entries: HashMap::new(),
            strategy,
            max_size: 100 * 1024 * 1024,
            current_size: 0,
            access_order: BinaryHeap::new(),
        })
    }

    async fn match_url_mut(&mut self, url: &str) -> Option<&mut CacheEntry> {
        self.entries.get_mut(url)
    }

    async fn put(&mut self, url: &str, entry: CacheEntry) -> Result<(), CacheError> {
        while self.current_size + entry.size > self.max_size && !self.entries.is_empty() {
            self.evict_lru().await?;
        }

        if entry.size > self.max_size {
            return Err(CacheError::QuotaExceeded);
        }

        if let Some(old_entry) = self.entries.remove(url) {
            self.current_size = self.current_size.saturating_sub(old_entry.size);
        }

        self.access_order.push(Reverse(AccessOrder {
            last_accessed: entry.last_accessed,
            url: url.to_string(),
        }));

        self.current_size += entry.size;
        self.entries.insert(url.to_string(), entry);

        Ok(())
    }

    async fn evict_lru(&mut self) -> Result<(), CacheError> {
        while let Some(Reverse(access_order)) = self.access_order.pop() {
            if let Some(entry) = self.entries.get(&access_order.url) {
                if entry.last_accessed == access_order.last_accessed {
                    let removed_entry = self.entries.remove(&access_order.url).unwrap();
                    self.current_size = self.current_size.saturating_sub(removed_entry.size);
                    return Ok(());
                }
            }
        }

        if let Some((url, _)) = self.entries.iter().next() {
            let url = url.clone();
            if let Some(entry) = self.entries.remove(&url) {
                self.current_size = self.current_size.saturating_sub(entry.size);
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
