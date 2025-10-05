pub mod fetch;

pub use fetch::FetchResponse;

use dashmap::DashMap;
use parking_lot::RwLock;
use reqwest::{header::HeaderMap, Client, ClientBuilder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::time::{timeout, Duration};
use url::Url;

use crate::BrowserConfig;

#[derive(Error, Debug)]
pub enum NetworkError {
    #[error("Request failed: {0}")]
    RequestFailed(String),
    #[error("Timeout: {0}")]
    Timeout(String),
    #[error("DNS resolution failed: {0}")]
    DnsResolution(String),
    #[error("SSL/TLS error: {0}")]
    SslError(String),
    #[error("Connection error: {0}")]
    Connection(String),
    #[error("Protocol error: {0}")]
    Protocol(String),
    #[error("Redirect error: {0}")]
    Redirect(String),
    #[error("Cache error: {0}")]
    Cache(String),
    #[error("Security policy violation: {0}")]
    SecurityPolicy(String),
}

pub type Result<T> = std::result::Result<T, NetworkError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub max_concurrent_requests: usize,
    pub request_timeout_ms: u64,
    pub dns_timeout_ms: u64,
    pub connect_timeout_ms: u64,
    pub max_redirects: usize,
    pub user_agent: String,
    pub enable_http2: bool,
    pub enable_brotli: bool,
    pub enable_gzip: bool,
    pub max_response_size_mb: usize,
    pub connection_pool_size: usize,
    pub keep_alive_timeout_s: u64,
    pub enable_dns_cache: bool,
    pub dns_cache_ttl_s: u64,
    pub enable_connection_reuse: bool,
    pub tcp_nodelay: bool,
    pub socket_timeout_ms: u64,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            max_concurrent_requests: 100,
            request_timeout_ms: 30000,
            dns_timeout_ms: 5000,
            connect_timeout_ms: 10000,
            max_redirects: 10,
            user_agent: "VulkanBrowser/1.0".to_string(),
            enable_http2: true,
            enable_brotli: true,
            enable_gzip: true,
            max_response_size_mb: 100,
            connection_pool_size: 50,
            keep_alive_timeout_s: 90,
            enable_dns_cache: true,
            dns_cache_ttl_s: 300,
            enable_connection_reuse: true,
            tcp_nodelay: true,
            socket_timeout_ms: 5000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CachePolicy {
    pub max_age: Option<u64>,
    pub must_revalidate: bool,
    pub no_cache: bool,
    pub no_store: bool,
    pub private: bool,
    pub public: bool,
    pub immutable: bool,
    pub stale_while_revalidate: Option<u64>,
    pub stale_if_error: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub data: Vec<u8>,
    pub headers: HeaderMap,
    pub cache_policy: CachePolicy,
    pub created_at: std::time::SystemTime,
    pub last_accessed: std::time::SystemTime,
    pub hit_count: u64,
    pub size: usize,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl CacheEntry {
    pub fn is_expired(&self) -> bool {
        if let Some(max_age) = self.cache_policy.max_age {
            if let Ok(elapsed) = self.created_at.elapsed() {
                return elapsed.as_secs() > max_age;
            }
        }
        false
    }

    pub fn is_stale(&self) -> bool {
        self.is_expired() || self.cache_policy.no_cache || self.cache_policy.must_revalidate
    }

    pub fn can_serve_stale(&self) -> bool {
        if let Some(stale_while_revalidate) = self.cache_policy.stale_while_revalidate {
            if let Ok(elapsed) = self.created_at.elapsed() {
                return elapsed.as_secs() <= stale_while_revalidate;
            }
        }
        false
    }
}

pub struct HttpCache {
    entries: Arc<DashMap<String, CacheEntry>>,
    max_size_bytes: usize,
    current_size_bytes: Arc<RwLock<usize>>,
    max_entries: usize,
}

impl HttpCache {
    pub fn new(max_size_bytes: usize, max_entries: usize) -> Self {
        Self {
            entries: Arc::new(DashMap::new()),
            max_size_bytes,
            current_size_bytes: Arc::new(RwLock::new(0)),
            max_entries,
        }
    }

    pub fn get(&self, key: &str) -> Option<CacheEntry> {
        if let Some(mut entry) = self.entries.get_mut(key) {
            entry.last_accessed = std::time::SystemTime::now();
            entry.hit_count += 1;
            Some(entry.clone())
        } else {
            None
        }
    }

    pub fn put(&self, key: String, entry: CacheEntry) {
        if entry.cache_policy.no_store {
            return;
        }

        self.ensure_capacity(entry.size);

        let mut current_size = self.current_size_bytes.write();
        *current_size += entry.size;
        drop(current_size);

        self.entries.insert(key, entry);
    }

    pub fn remove(&self, key: &str) -> Option<CacheEntry> {
        if let Some((_, entry)) = self.entries.remove(key) {
            let mut current_size = self.current_size_bytes.write();
            *current_size = current_size.saturating_sub(entry.size);
            Some(entry)
        } else {
            None
        }
    }

    pub fn clear(&self) {
        self.entries.clear();
        *self.current_size_bytes.write() = 0;
    }

    fn ensure_capacity(&self, needed_size: usize) {
        let current_size = *self.current_size_bytes.read();

        if current_size + needed_size > self.max_size_bytes
            || self.entries.len() >= self.max_entries
        {
            self.evict_entries(needed_size);
        }
    }

    fn evict_entries(&self, needed_size: usize) {
        let mut entries_to_remove = Vec::new();
        let mut freed_size = 0;

        // Collect entries sorted by last accessed time (LRU)
        let mut entries: Vec<_> = self
            .entries
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();

        entries.sort_by_key(|(_, entry)| entry.last_accessed);

        for (key, entry) in entries {
            if freed_size >= needed_size {
                break;
            }

            entries_to_remove.push(key);
            freed_size += entry.size;
        }

        for key in entries_to_remove {
            self.remove(&key);
        }
    }

    pub fn get_stats(&self) -> CacheStats {
        let current_size = *self.current_size_bytes.read();
        let entry_count = self.entries.len();
        let total_hits: u64 = self.entries.iter().map(|entry| entry.hit_count).sum();

        CacheStats {
            entry_count,
            total_size_bytes: current_size,
            max_size_bytes: self.max_size_bytes,
            hit_count: total_hits,
            utilization: current_size as f64 / self.max_size_bytes as f64,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub entry_count: usize,
    pub total_size_bytes: usize,
    pub max_size_bytes: usize,
    pub hit_count: u64,
    pub utilization: f64,
}

pub struct ConnectionPool {
    clients: Arc<DashMap<String, Client>>,
    max_connections_per_host: usize,
    #[allow(dead_code)]
    connection_timeout: Duration,
}

impl ConnectionPool {
    pub fn new(max_connections_per_host: usize, connection_timeout: Duration) -> Self {
        Self {
            clients: Arc::new(DashMap::new()),
            max_connections_per_host,
            connection_timeout,
        }
    }

    pub fn get_client(&self, host: &str, config: &NetworkConfig) -> Result<Client> {
        if let Some(client) = self.clients.get(host) {
            return Ok(client.clone());
        }

        let client = ClientBuilder::new()
            .timeout(Duration::from_millis(config.request_timeout_ms))
            .connect_timeout(Duration::from_millis(config.connect_timeout_ms))
            .pool_max_idle_per_host(self.max_connections_per_host)
            .pool_idle_timeout(Duration::from_secs(config.keep_alive_timeout_s))
            .user_agent(&config.user_agent)
            .gzip(config.enable_gzip)
            .brotli(config.enable_brotli)
            .http2_prior_knowledge()
            .tcp_nodelay(config.tcp_nodelay)
            .redirect(reqwest::redirect::Policy::limited(config.max_redirects))
            .build()
            .map_err(|e| NetworkError::Connection(e.to_string()))?;

        self.clients.insert(host.to_string(), client.clone());
        Ok(client)
    }

    pub fn remove_client(&self, host: &str) {
        self.clients.remove(host);
    }

    pub fn clear(&self) {
        self.clients.clear();
    }

    pub fn get_stats(&self) -> ConnectionPoolStats {
        ConnectionPoolStats {
            active_connections: self.clients.len(),
            max_connections_per_host: self.max_connections_per_host,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConnectionPoolStats {
    pub active_connections: usize,
    pub max_connections_per_host: usize,
}

pub struct DnsCache {
    entries: Arc<DashMap<String, DnsCacheEntry>>,
    ttl: Duration,
}

#[derive(Debug, Clone)]
struct DnsCacheEntry {
    resolved_ips: Vec<std::net::IpAddr>,
    created_at: std::time::Instant,
}

impl DnsCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            entries: Arc::new(DashMap::new()),
            ttl,
        }
    }

    pub fn get(&self, hostname: &str) -> Option<Vec<std::net::IpAddr>> {
        if let Some(entry) = self.entries.get(hostname) {
            if entry.created_at.elapsed() < self.ttl {
                return Some(entry.resolved_ips.clone());
            } else {
                self.entries.remove(hostname);
            }
        }
        None
    }

    pub fn put(&self, hostname: String, ips: Vec<std::net::IpAddr>) {
        let entry = DnsCacheEntry {
            resolved_ips: ips,
            created_at: std::time::Instant::now(),
        };
        self.entries.insert(hostname, entry);
    }

    pub fn clear(&self) {
        self.entries.clear();
    }

    pub fn cleanup_expired(&self) {
        let expired_keys: Vec<String> = self
            .entries
            .iter()
            .filter(|entry| entry.created_at.elapsed() >= self.ttl)
            .map(|entry| entry.key().clone())
            .collect();

        for key in expired_keys {
            self.entries.remove(&key);
        }
    }
}

#[derive(Debug, Clone)]
pub struct NetworkMetrics {
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub total_bytes_downloaded: u64,
    pub total_bytes_uploaded: u64,
    pub average_request_time_ms: f64,
    pub dns_resolution_time_ms: f64,
    pub connection_time_ms: f64,
    pub ssl_handshake_time_ms: f64,
    pub first_byte_time_ms: f64,
    pub active_connections: usize,
}

impl Default for NetworkMetrics {
    fn default() -> Self {
        Self {
            total_requests: 0,
            successful_requests: 0,
            failed_requests: 0,
            cache_hits: 0,
            cache_misses: 0,
            total_bytes_downloaded: 0,
            total_bytes_uploaded: 0,
            average_request_time_ms: 0.0,
            dns_resolution_time_ms: 0.0,
            connection_time_ms: 0.0,
            ssl_handshake_time_ms: 0.0,
            first_byte_time_ms: 0.0,
            active_connections: 0,
        }
    }
}

pub struct RequestLimiter {
    semaphore: Arc<tokio::sync::Semaphore>,
    max_concurrent: usize,
}

impl RequestLimiter {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            semaphore: Arc::new(tokio::sync::Semaphore::new(max_concurrent)),
            max_concurrent,
        }
    }

    pub async fn acquire(&self) -> tokio::sync::SemaphorePermit<'_> {
        self.semaphore.acquire().await.expect("Semaphore closed")
    }

    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    pub fn max_permits(&self) -> usize {
        self.max_concurrent
    }
}

pub struct SecurityPolicy {
    pub allowed_schemes: Vec<String>,
    pub blocked_hosts: Vec<String>,
    pub allowed_hosts: Option<Vec<String>>,
    pub max_request_size: usize,
    pub max_response_size: usize,
    pub require_https_for_sensitive: bool,
    pub block_private_ips: bool,
    pub block_localhost: bool,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            allowed_schemes: vec!["http".to_string(), "https".to_string()],
            blocked_hosts: Vec::new(),
            allowed_hosts: None,
            max_request_size: 10 * 1024 * 1024,   // 10 MB
            max_response_size: 100 * 1024 * 1024, // 100 MB
            require_https_for_sensitive: true,
            block_private_ips: false,
            block_localhost: false,
        }
    }
}

impl SecurityPolicy {
    pub fn check_url(&self, url: &Url) -> Result<()> {
        // Check scheme
        if !self.allowed_schemes.contains(&url.scheme().to_string()) {
            return Err(NetworkError::SecurityPolicy(format!(
                "Scheme '{}' not allowed",
                url.scheme()
            )));
        }

        // Check host against blocklist
        if let Some(host) = url.host_str() {
            if self.blocked_hosts.contains(&host.to_string()) {
                return Err(NetworkError::SecurityPolicy(format!(
                    "Host '{}' is blocked",
                    host
                )));
            }

            // Check host against allowlist if specified
            if let Some(ref allowed) = self.allowed_hosts {
                if !allowed.contains(&host.to_string()) {
                    return Err(NetworkError::SecurityPolicy(format!(
                        "Host '{}' not in allowlist",
                        host
                    )));
                }
            }

            // Check for private IPs if blocked
            if self.block_private_ips {
                if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                    if self.is_private_ip(&ip) {
                        return Err(NetworkError::SecurityPolicy(
                            "Private IP addresses are blocked".to_string(),
                        ));
                    }
                }
            }

            // Check for localhost if blocked
            if self.block_localhost && (host == "localhost" || host == "127.0.0.1" || host == "::1")
            {
                return Err(NetworkError::SecurityPolicy(
                    "Localhost access is blocked".to_string(),
                ));
            }
        }

        Ok(())
    }

    fn is_private_ip(&self, ip: &std::net::IpAddr) -> bool {
        match ip {
            std::net::IpAddr::V4(ipv4) => {
                ipv4.is_private() || ipv4.is_loopback() || ipv4.is_link_local()
            }
            std::net::IpAddr::V6(ipv6) => {
                let octets = ipv6.octets();
                let prefix = u16::from_be_bytes([octets[0], octets[1]]);
                let is_unique_local = (prefix & 0xfe00) == 0xfc00;
                let is_link_local = (prefix & 0xffc0) == 0xfe80;

                ipv6.is_loopback() || is_unique_local || is_link_local
            }
        }
    }
}

pub struct NetworkManager {
    config: NetworkConfig,
    http_cache: Arc<HttpCache>,
    connection_pool: Arc<ConnectionPool>,
    dns_cache: Arc<DnsCache>,
    request_limiter: Arc<RequestLimiter>,
    security_policy: Arc<SecurityPolicy>,
    metrics: Arc<RwLock<NetworkMetrics>>,
    active_requests: Arc<DashMap<String, tokio::sync::oneshot::Sender<()>>>,
}

impl NetworkManager {
    pub async fn new(browser_config: &BrowserConfig) -> Result<Self> {
        let config = NetworkConfig {
            user_agent: browser_config.user_agent.clone(),
            max_concurrent_requests: if browser_config.max_processes > 0 {
                browser_config.max_processes * 10
            } else {
                100
            },
            ..NetworkConfig::default()
        };

        let http_cache = Arc::new(HttpCache::new(
            50 * 1024 * 1024, // 50 MB cache
            10000,            // Max 10k entries
        ));

        let connection_pool = Arc::new(ConnectionPool::new(
            config.connection_pool_size,
            Duration::from_millis(config.connect_timeout_ms),
        ));

        let dns_cache = Arc::new(DnsCache::new(Duration::from_secs(config.dns_cache_ttl_s)));

        let request_limiter = Arc::new(RequestLimiter::new(config.max_concurrent_requests));

        Ok(Self {
            config,
            http_cache,
            connection_pool,
            dns_cache,
            request_limiter,
            security_policy: Arc::new(SecurityPolicy::default()),
            metrics: Arc::new(RwLock::new(NetworkMetrics::default())),
            active_requests: Arc::new(DashMap::new()),
        })
    }

    pub async fn fetch(&self, url: &str) -> Result<String> {
        let request = FetchRequest {
            url: url.to_string(),
            method: "GET".to_string(),
            headers: HashMap::new(),
            body: None,
            timeout_ms: Some(self.config.request_timeout_ms),
            follow_redirects: true,
            cache_policy: Some(CachePolicy::default()),
        };

        let response = self.fetch_with_request(request).await?;
        String::from_utf8(response.body)
            .map_err(|e| NetworkError::Protocol(format!("Invalid UTF-8: {}", e)))
    }

    pub async fn fetch_with_request(&self, request: FetchRequest) -> Result<FetchResponse> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let start_time = std::time::Instant::now();

        // Acquire request limiter permit
        let _permit = self.request_limiter.acquire().await;

        // Parse URL
        let url = Url::parse(&request.url)
            .map_err(|e| NetworkError::RequestFailed(format!("Invalid URL: {}", e)))?;

        // Check security policy
        self.security_policy.check_url(&url)?;

        // Update metrics
        {
            let mut metrics = self.metrics.write();
            metrics.total_requests += 1;
        }

        // Check cache first
        if let Some(cache_policy) = &request.cache_policy {
            if !cache_policy.no_cache {
                if let Some(cached_response) = self.get_cached_response(&request.url) {
                    if !cached_response.is_stale() || cached_response.can_serve_stale() {
                        let mut metrics = self.metrics.write();
                        metrics.cache_hits += 1;

                        return Ok(FetchResponse {
                            status: 200,
                            headers: cached_response
                                .headers
                                .iter()
                                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                                .collect(),
                            body: cached_response.data,
                            url: request.url,
                            redirected: true,
                        });
                    }
                }
            }
        }

        // Cache miss
        {
            let mut metrics = self.metrics.write();
            metrics.cache_misses += 1;
        }

        // Create cancellation token for this request
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        self.active_requests.insert(request_id.clone(), cancel_tx);

        // Perform the actual request
        let result = self.perform_request(request, cancel_rx).await;

        // Clean up
        self.active_requests.remove(&request_id);

        // Update metrics
        let request_time = start_time.elapsed();
        {
            let mut metrics = self.metrics.write();
            match &result {
                Ok(response) => {
                    metrics.successful_requests += 1;
                    metrics.total_bytes_downloaded += response.body.len() as u64;
                }
                Err(_) => {
                    metrics.failed_requests += 1;
                }
            }

            // Update average request time
            let total_requests = metrics.total_requests as f64;
            metrics.average_request_time_ms = (metrics.average_request_time_ms
                * (total_requests - 1.0)
                + request_time.as_millis() as f64)
                / total_requests;
        }

        result
    }

    async fn perform_request(
        &self,
        request: FetchRequest,
        mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<FetchResponse> {
        let url = Url::parse(&request.url)
            .map_err(|e| NetworkError::RequestFailed(format!("Invalid URL: {}", e)))?;

        let host = url.host_str().unwrap_or("localhost");
        let client = self.connection_pool.get_client(host, &self.config)?;

        let mut req_builder = match request.method.as_str() {
            "GET" => client.get(&request.url),
            "POST" => client.post(&request.url),
            "PUT" => client.put(&request.url),
            "DELETE" => client.delete(&request.url),
            "HEAD" => client.head(&request.url),
            "PATCH" => client.patch(&request.url),
            _ => {
                return Err(NetworkError::RequestFailed(format!(
                    "Unsupported method: {}",
                    request.method
                )))
            }
        };

        // Add headers
        for (key, value) in request.headers {
            req_builder = req_builder.header(&key, &value);
        }

        // Add body if present
        if let Some(body) = request.body {
            req_builder = req_builder.body(body);
        }

        // Set timeout
        let timeout_duration =
            Duration::from_millis(request.timeout_ms.unwrap_or(self.config.request_timeout_ms));

        // Execute request with timeout and cancellation
        let request_future = req_builder.send();
        let timeout_future = timeout(timeout_duration, request_future);

        let response = tokio::select! {
            _ = &mut cancel_rx => {
                return Err(NetworkError::RequestFailed("Request cancelled".to_string()));
            }
            result = timeout_future => {
                match result {
                    Ok(Ok(response)) => response,
                    Ok(Err(e)) => return Err(NetworkError::RequestFailed(e.to_string())),
                    Err(_) => return Err(NetworkError::Timeout("Request timeout".to_string())),
                }
            }
        };

        // Read response body
        let status = response.status().as_u16();
        let headers: HashMap<String, String> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        let body = response
            .bytes()
            .await
            .map_err(|e| NetworkError::RequestFailed(format!("Failed to read body: {}", e)))?
            .to_vec();

        // Check response size limit
        if body.len() > self.config.max_response_size_mb * 1024 * 1024 {
            return Err(NetworkError::RequestFailed(
                "Response too large".to_string(),
            ));
        }

        let fetch_response = FetchResponse {
            status,
            headers: headers.clone(),
            body: body.clone(),
            url: request.url.clone(),
            redirected: false,
        };

        // Cache the response if appropriate
        if let Some(cache_policy) = request.cache_policy {
            if !cache_policy.no_store && status == 200 {
                self.cache_response(&request.url, &fetch_response, cache_policy);
            }
        }

        Ok(fetch_response)
    }

    fn get_cached_response(&self, url: &str) -> Option<CacheEntry> {
        self.http_cache.get(url)
    }

    fn cache_response(&self, url: &str, response: &FetchResponse, cache_policy: CachePolicy) {
        let mut headers = HeaderMap::new();
        for (key, value) in &response.headers {
            if let (Ok(header_name), Ok(header_value)) = (
                key.parse::<reqwest::header::HeaderName>(),
                value.parse::<reqwest::header::HeaderValue>(),
            ) {
                headers.insert(header_name, header_value);
            }
        }

        let cache_entry = CacheEntry {
            data: response.body.clone(),
            headers,
            cache_policy,
            created_at: std::time::SystemTime::now(),
            last_accessed: std::time::SystemTime::now(),
            hit_count: 0,
            size: response.body.len(),
            etag: response.headers.get("etag").cloned(),
            last_modified: response.headers.get("last-modified").cloned(),
        };

        self.http_cache.put(url.to_string(), cache_entry);
    }

    pub async fn cancel_request(&self, request_id: &str) -> bool {
        if let Some((_, cancel_tx)) = self.active_requests.remove(request_id) {
            let _ = cancel_tx.send(());
            true
        } else {
            false
        }
    }

    pub async fn cancel_all_requests(&self) {
        let request_ids: Vec<String> = self
            .active_requests
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        for request_id in request_ids {
            self.cancel_request(&request_id).await;
        }
    }

    pub fn clear_cache(&self) {
        self.http_cache.clear();
    }

    pub fn clear_dns_cache(&self) {
        self.dns_cache.clear();
    }

    pub fn get_metrics(&self) -> NetworkMetrics {
        let mut metrics = self.metrics.read().clone();
        metrics.active_connections = self.connection_pool.get_stats().active_connections;
        metrics
    }

    pub fn get_cache_stats(&self) -> CacheStats {
        self.http_cache.get_stats()
    }

    pub fn update_security_policy(&self, _policy: SecurityPolicy) {
        // In a real implementation, this would need proper synchronization
        // For now, we'll note that this is a design limitation
    }

    pub async fn shutdown(&self) -> Result<()> {
        // Cancel all active requests
        self.cancel_all_requests().await;

        // Clear all caches
        self.clear_cache();
        self.clear_dns_cache();
        self.connection_pool.clear();

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub url: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Vec<u8>>,
    pub timeout_ms: Option<u64>,
    pub follow_redirects: bool,
    pub cache_policy: Option<CachePolicy>,
}
