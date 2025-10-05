#![allow(async_fn_in_trait)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
pub struct CacheResult {
    pub entry: CacheEntry,
    pub is_stale: bool,
}

pub trait CacheOperations: Send + Sync {
    async fn get_entry(&mut self, url: &str) -> Option<CacheResult>;
    async fn put_entry(&mut self, url: &str, entry: CacheEntry) -> Result<(), CacheError>;
}

pub trait NetworkClient: Send + Sync {
    fn fetch(
        &self,
        request: FetchRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FetchResponse, NetworkError>> + Send + '_>>;
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum CacheStrategy {
    #[default]
    CacheFirst,
    NetworkFirst,
    NetworkOnly,
    CacheOnly,
    StaleWhileRevalidate,
}

pub struct StrategyExecutor<T: CacheOperations> {
    cache: T,
    network: Arc<dyn NetworkClient>,
    config: StrategyConfig,
}

#[derive(Debug, Clone)]
pub struct StrategyConfig {
    pub network_timeout: Duration,
    pub stale_threshold: Duration,
    pub retry_attempts: u32,
    pub background_refresh: bool,
    pub max_cache_age: Duration,
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

#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("Request timeout after {0:?}")]
    Timeout(Duration),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Invalid response: {0}")]
    InvalidResponse(String),
    #[error("Too many redirects")]
    TooManyRedirects,
    #[error("Invalid HTTP method: {0}")]
    InvalidMethod(String),
    #[error("DNS resolution failed")]
    DnsFailure,
    #[error("Connection refused")]
    ConnectionRefused,
}

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("IO error: {0}")]
    IoError(String),
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("Quota exceeded")]
    QuotaExceeded,
    #[error("Invalid entry")]
    InvalidEntry,
}

#[derive(Debug, thiserror::Error)]
pub enum StrategyError {
    #[error("Cache miss")]
    CacheMiss,
    #[error("Network error: {0}")]
    Network(#[from] NetworkError),
    #[error("Cache error: {0}")]
    Cache(#[from] CacheError),
    #[error("No fallback available")]
    NoFallback,
    #[error("Request cancelled")]
    Cancelled,
}

pub struct ReqwestNetworkClient {
    client: reqwest::Client,
    timeout: Duration,
    max_retries: u32,
}

pub enum NetworkClientType {
    Reqwest(ReqwestNetworkClient),
    Mock(MockNetworkClient),
}

pub struct MockNetworkClient {
    responses: HashMap<String, FetchResponse>,
    delay: Duration,
}

impl<T: CacheOperations> StrategyExecutor<T> {
    pub fn new(cache: T, network: Arc<dyn NetworkClient>, config: StrategyConfig) -> Self {
        Self {
            cache,
            network,
            config,
        }
    }

    pub async fn execute(
        &mut self,
        strategy: &CacheStrategy,
        request: &FetchRequest,
    ) -> Result<FetchResponse, StrategyError> {
        match strategy {
            CacheStrategy::CacheFirst => self.cache_first_strategy(request).await,
            CacheStrategy::NetworkFirst => self.network_first_strategy(request).await,
            CacheStrategy::NetworkOnly => self.network_only_strategy(request).await,
            CacheStrategy::CacheOnly => self.cache_only_strategy(request).await,
            CacheStrategy::StaleWhileRevalidate => {
                self.stale_while_revalidate_strategy(request).await
            }
        }
    }

    async fn cache_first_strategy(
        &mut self,
        request: &FetchRequest,
    ) -> Result<FetchResponse, StrategyError> {
        if let Some(cache_result) = self.cache.get_entry(&request.url).await {
            if !cache_result.is_stale {
                let response = Self::entry_to_response(&cache_result.entry);
                let updated_entry = self.update_access_stats(cache_result.entry);
                let _ = self.cache.put_entry(&request.url, updated_entry).await;
                return Ok(response);
            }
        }

        match self.fetch_with_retry(request).await {
            Ok(response) => {
                self.maybe_cache_response(request, &response).await;
                Ok(response)
            }
            Err(e) => Err(e),
        }
    }

    async fn network_first_strategy(
        &mut self,
        request: &FetchRequest,
    ) -> Result<FetchResponse, StrategyError> {
        match self.fetch_with_retry(request).await {
            Ok(response) => {
                self.maybe_cache_response(request, &response).await;
                Ok(response)
            }
            Err(_) => {
                if let Some(cache_result) = self.cache.get_entry(&request.url).await {
                    let response = Self::entry_to_response(&cache_result.entry);
                    let updated_entry = self.update_access_stats(cache_result.entry);
                    let _ = self.cache.put_entry(&request.url, updated_entry).await;
                    Ok(response)
                } else {
                    Err(StrategyError::NoFallback)
                }
            }
        }
    }

    async fn network_only_strategy(
        &self,
        request: &FetchRequest,
    ) -> Result<FetchResponse, StrategyError> {
        self.fetch_with_retry(request).await
    }

    async fn cache_only_strategy(
        &mut self,
        request: &FetchRequest,
    ) -> Result<FetchResponse, StrategyError> {
        if let Some(cache_result) = self.cache.get_entry(&request.url).await {
            let response = Self::entry_to_response(&cache_result.entry);
            let updated_entry = self.update_access_stats(cache_result.entry);
            let _ = self.cache.put_entry(&request.url, updated_entry).await;
            Ok(response)
        } else {
            Err(StrategyError::CacheMiss)
        }
    }

    async fn stale_while_revalidate_strategy(
        &mut self,
        request: &FetchRequest,
    ) -> Result<FetchResponse, StrategyError> {
        if let Some(cache_result) = self.cache.get_entry(&request.url).await {
            let response = Self::entry_to_response(&cache_result.entry);

            if cache_result.is_stale && self.config.background_refresh {
                let network = Arc::clone(&self.network);
                let request_clone = request.clone();

                tokio::spawn(async move {
                    let _ = network.fetch(request_clone).await;
                });
            }

            let updated_entry = self.update_access_stats(cache_result.entry);
            let _ = self.cache.put_entry(&request.url, updated_entry).await;
            Ok(response)
        } else {
            match self.fetch_with_retry(request).await {
                Ok(response) => {
                    self.maybe_cache_response(request, &response).await;
                    Ok(response)
                }
                Err(e) => Err(e),
            }
        }
    }

    async fn fetch_with_retry(
        &self,
        request: &FetchRequest,
    ) -> Result<FetchResponse, StrategyError> {
        let mut last_error = None;

        for attempt in 0..=self.config.retry_attempts {
            match self.network.fetch(request.clone()).await {
                Ok(response) => return Ok(response),
                Err(err) => {
                    last_error = Some(err);
                    if attempt < self.config.retry_attempts {
                        let delay = Duration::from_millis(100 * (1 << attempt.min(6)));
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(StrategyError::Network(last_error.unwrap()))
    }

    async fn maybe_cache_response(&mut self, request: &FetchRequest, response: &FetchResponse) {
        if Self::is_cacheable(response) {
            let entry = CacheEntry {
                url: request.url.clone(),
                response: CachedResponse {
                    status: response.status,
                    headers: response.headers.clone(),
                    body: response.body.clone(),
                },
                stored_at: SystemTime::now(),
                expires_at: Self::calculate_expiry(&response.headers),
                size: response.body.len() as u64,
                access_count: 1,
                last_accessed: SystemTime::now(),
            };

            let _ = self.cache.put_entry(&request.url, entry).await;
        }
    }

    fn update_access_stats(&self, mut entry: CacheEntry) -> CacheEntry {
        entry.access_count = entry.access_count.saturating_add(1);
        entry.last_accessed = SystemTime::now();
        entry
    }

    fn is_cacheable(response: &FetchResponse) -> bool {
        match response.status {
            200..=299 => true,
            304 => true,
            _ => false,
        }
    }

    fn calculate_expiry(headers: &HashMap<String, String>) -> Option<SystemTime> {
        if let Some(cache_control) = headers
            .get("cache-control")
            .or_else(|| headers.get("Cache-Control"))
        {
            if cache_control.contains("no-cache") || cache_control.contains("no-store") {
                return None;
            }

            if let Some(max_age) = Self::extract_max_age(cache_control) {
                return Some(SystemTime::now() + Duration::from_secs(max_age));
            }
        }

        if let Some(expires_header) = headers.get("expires").or_else(|| headers.get("Expires")) {
            if let Some(expires_time) = Self::parse_http_date(expires_header) {
                return Some(expires_time);
            }
        }

        Some(SystemTime::now() + Duration::from_secs(300))
    }

    fn extract_max_age(cache_control: &str) -> Option<u64> {
        cache_control.split(',').find_map(|directive| {
            let directive = directive.trim();
            if directive.starts_with("max-age=") {
                directive[8..].trim().parse().ok()
            } else {
                None
            }
        })
    }

    fn parse_http_date(date_str: &str) -> Option<SystemTime> {
        let formats = [
            "%a, %d %b %Y %H:%M:%S GMT",
            "%A, %d-%b-%y %H:%M:%S GMT",
            "%a %b %e %H:%M:%S %Y",
        ];

        for format in &formats {
            if let Ok(datetime) = chrono::DateTime::parse_from_str(date_str, format) {
                let timestamp = datetime.timestamp();
                if timestamp >= 0 {
                    return Some(std::time::UNIX_EPOCH + Duration::from_secs(timestamp as u64));
                }
            }
        }

        None
    }

    fn entry_to_response(entry: &CacheEntry) -> FetchResponse {
        FetchResponse {
            status: entry.response.status,
            headers: entry.response.headers.clone(),
            body: entry.response.body.clone(),
        }
    }
}

impl ReqwestNetworkClient {
    pub fn new(timeout: Duration) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::limited(5))
            .user_agent("VulkanBrowser/1.0")
            .pool_max_idle_per_host(10)
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            timeout,
            max_retries: 3,
        }
    }

    pub fn with_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }
}

impl NetworkClient for ReqwestNetworkClient {
    fn fetch(
        &self,
        request: FetchRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FetchResponse, NetworkError>> + Send + '_>> {
        Box::pin(async move {
            let method = match request.method.as_str() {
                "GET" => reqwest::Method::GET,
                "POST" => reqwest::Method::POST,
                "PUT" => reqwest::Method::PUT,
                "DELETE" => reqwest::Method::DELETE,
                "PATCH" => reqwest::Method::PATCH,
                "HEAD" => reqwest::Method::HEAD,
                "OPTIONS" => reqwest::Method::OPTIONS,
                m => return Err(NetworkError::InvalidMethod(m.to_string())),
            };

            let mut req_builder = self.client.request(method, &request.url);

            for (key, value) in &request.headers {
                if let (Ok(name), Ok(value)) = (
                    reqwest::header::HeaderName::from_bytes(key.as_bytes()),
                    reqwest::header::HeaderValue::from_str(value),
                ) {
                    req_builder = req_builder.header(name, value);
                }
            }

            if let Some(body) = &request.body {
                req_builder = req_builder.body(body.clone());
            }

            let response = tokio::time::timeout(self.timeout, req_builder.send())
                .await
                .map_err(|_| NetworkError::Timeout(self.timeout))?
                .map_err(|e| {
                    if e.is_timeout() {
                        NetworkError::Timeout(self.timeout)
                    } else if e.is_redirect() {
                        NetworkError::TooManyRedirects
                    } else if e.is_connect() {
                        NetworkError::ConnectionRefused
                    } else if e.is_request() {
                        NetworkError::InvalidResponse(e.to_string())
                    } else {
                        NetworkError::Network(e.to_string())
                    }
                })?;

            let status = response.status().as_u16();

            let headers = response
                .headers()
                .iter()
                .filter_map(|(k, v)| {
                    v.to_str()
                        .ok()
                        .map(|value| (k.to_string(), value.to_string()))
                })
                .collect();

            let body = tokio::time::timeout(self.timeout, response.bytes())
                .await
                .map_err(|_| NetworkError::Timeout(self.timeout))?
                .map_err(|e| NetworkError::Network(e.to_string()))?
                .to_vec();

            Ok(FetchResponse {
                status,
                headers,
                body,
            })
        })
    }
}

impl Default for MockNetworkClient {
    fn default() -> Self {
        Self::new()
    }
}

impl MockNetworkClient {
    pub fn new() -> Self {
        Self {
            responses: HashMap::new(),
            delay: Duration::from_millis(10),
        }
    }

    pub fn with_response(mut self, url: String, response: FetchResponse) -> Self {
        self.responses.insert(url, response);
        self
    }

    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

impl NetworkClient for MockNetworkClient {
    fn fetch(
        &self,
        request: FetchRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FetchResponse, NetworkError>> + Send + '_>> {
        let delay = self.delay;
        let response = self.responses.get(&request.url).cloned();

        Box::pin(async move {
            tokio::time::sleep(delay).await;

            response.ok_or_else(|| {
                NetworkError::Network(format!("No mock response for {}", request.url))
            })
        })
    }
}

impl CacheEntry {
    pub fn is_stale(&self, threshold: Duration) -> bool {
        if let Some(expires_at) = self.expires_at {
            return SystemTime::now() > expires_at;
        }

        SystemTime::now()
            .duration_since(self.stored_at)
            .map(|age| age > threshold)
            .unwrap_or(false)
    }

    pub fn should_revalidate(&self, threshold: Duration) -> bool {
        SystemTime::now()
            .duration_since(self.last_accessed)
            .map(|since_access| since_access > threshold)
            .unwrap_or(true)
    }

    pub fn access_frequency(&self) -> f64 {
        let age = SystemTime::now()
            .duration_since(self.stored_at)
            .unwrap_or_default()
            .as_secs() as f64;

        if age > 0.0 {
            self.access_count as f64 / age
        } else {
            0.0
        }
    }
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            network_timeout: Duration::from_secs(30),
            stale_threshold: Duration::from_secs(3600),
            retry_attempts: 3,
            background_refresh: true,
            max_cache_age: Duration::from_secs(86400),
        }
    }
}
