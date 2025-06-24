use reqwest::{Client};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::timeout;
use url::Url;

#[derive(Debug, Clone)]
pub struct FetchOptions {
    pub method: HttpMethod,
    pub headers: HashMap<String, String>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
    pub follow_redirects: bool,
    pub credentials: CredentialsMode,
    pub cache: CacheMode,
    pub cors_mode: CorsMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
}

#[derive(Debug, Clone)]
pub enum CredentialsMode {
    Omit,
    SameOrigin,
    Include,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CacheMode {
    Default,
    NoStore,
    Reload,
    NoCache,
    ForceCache,
    OnlyIfCached,
}

#[derive(Debug, Clone)]
pub enum CorsMode {
    SameOrigin,
    Cors,
    NoCors,
}

pub struct FetchResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub url: String,
    pub redirected: bool,
}

pub struct FetchEngine {
    client: Client,
    cache: ResponseCache,
}

impl FetchEngine {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("VulkanRenderer/1.0")
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            cache: ResponseCache::new(),
        }
    }

    pub async fn fetch(&self, url: &str, options: FetchOptions) -> Result<FetchResponse, FetchError> {
        let parsed_url = Url::parse(url).map_err(FetchError::InvalidUrl)?;
        
        if let Some(cached) = self.cache.get(&parsed_url, &options).await {
            return Ok(cached);
        }

        let response = self.execute_request(&parsed_url, &options).await?;
        
        if options.cache != CacheMode::NoStore {
            self.cache.store(&parsed_url, &options, &response).await;
        }

        Ok(response)
    }

    async fn execute_request(&self, url: &Url, options: &FetchOptions) -> Result<FetchResponse, FetchError> {
        let mut request = match options.method {
            HttpMethod::Get => self.client.get(url.as_str()),
            HttpMethod::Post => self.client.post(url.as_str()),
            HttpMethod::Put => self.client.put(url.as_str()),
            HttpMethod::Delete => self.client.delete(url.as_str()),
            HttpMethod::Patch => self.client.patch(url.as_str()),
            HttpMethod::Head => self.client.head(url.as_str()),
            HttpMethod::Options => self.client.request(reqwest::Method::OPTIONS, url.as_str()),
        };

        for (key, value) in &options.headers {
            request = request.header(key, value);
        }

        if let Some(body) = &options.body {
            request = request.body(body.clone());
        }

        let response = timeout(options.timeout, request.send())
            .await
            .map_err(|_| FetchError::Timeout)?
            .map_err(FetchError::RequestError)?;

        let status = response.status().as_u16();
        let headers = response.headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        
        let final_url = response.url().clone();
        let redirected = final_url != *url;
        
        let body = response.bytes()
            .await
            .map_err(FetchError::RequestError)?
            .to_vec();

        Ok(FetchResponse {
            status,
            headers,
            body,
            url: final_url.to_string(),
            redirected,
        })
    }
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            method: HttpMethod::Get,
            headers: HashMap::new(),
            body: None,
            timeout: Duration::from_secs(30),
            follow_redirects: true,
            credentials: CredentialsMode::SameOrigin,
            cache: CacheMode::Default,
            cors_mode: CorsMode::Cors,
        }
    }
}

struct ResponseCache {
    cache: std::sync::Arc<tokio::sync::RwLock<HashMap<String, CacheEntry>>>,
}

struct CacheEntry {
    response: FetchResponse,
    expires: std::time::Instant,
    etag: Option<String>,
}

impl ResponseCache {
    fn new() -> Self {
        Self {
            cache: std::sync::Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    async fn get(&self, url: &Url, options: &FetchOptions) -> Option<FetchResponse> {
        if matches!(options.cache, CacheMode::NoStore | CacheMode::Reload) {
            return None;
        }

        let cache = self.cache.read().await;
        let key = self.cache_key(url, options);
        
        if let Some(entry) = cache.get(&key) {
            if entry.expires > std::time::Instant::now() {
                return Some(FetchResponse {
                    status: entry.response.status,
                    headers: entry.response.headers.clone(),
                    body: entry.response.body.clone(),
                    url: entry.response.url.clone(),
                    redirected: entry.response.redirected,
                });
            }
        }

        None
    }

    async fn store(&self, url: &Url, options: &FetchOptions, response: &FetchResponse) {
        let mut cache = self.cache.write().await;
        let key = self.cache_key(url, options);
        
        let expires = std::time::Instant::now() + Duration::from_secs(300);
        let etag = response.headers.get("etag").cloned();
        
        cache.insert(key, CacheEntry {
            response: FetchResponse {
                status: response.status,
                headers: response.headers.clone(),
                body: response.body.clone(),
                url: response.url.clone(),
                redirected: response.redirected,
            },
            expires,
            etag,
        });
    }

    fn cache_key(&self, url: &Url, options: &FetchOptions) -> String {
        format!("{}:{:?}", url, options.method)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("Invalid URL: {0}")]
    InvalidUrl(url::ParseError),
    #[error("Request timeout")]
    Timeout,
    #[error("Request error: {0}")]
    RequestError(reqwest::Error),
    #[error("Network error: {0}")]
    NetworkError(String),
}

impl Default for FetchEngine {
    fn default() -> Self {
        Self::new()
    }
}