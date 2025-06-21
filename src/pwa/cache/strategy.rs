use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CacheStrategy {
    CacheFirst,
    NetworkFirst,
    NetworkOnly,
    CacheOnly,
    StaleWhileRevalidate,
}

impl CacheStrategy {
    pub async fn execute(&self, request: &crate::pwa::FetchRequest, cache: &crate::pwa::cache::Cache) -> Result<crate::pwa::FetchResponse, crate::pwa::cache::CacheError> {
        match self {
            CacheStrategy::CacheFirst => {
                if let Some(cached) = cache.match_url(&request.url).await {
                    return Ok(crate::pwa::FetchResponse {
                        status: cached.response.status,
                        headers: cached.response.headers.clone(),
                        body: cached.response.body.clone(),
                    });
                }
                
                self.fetch_from_network(request).await
            }
            
            CacheStrategy::NetworkFirst => {
                match self.fetch_from_network(request).await {
                    Ok(response) => Ok(response),
                    Err(_) => {
                        if let Some(cached) = cache.match_url(&request.url).await {
                            Ok(crate::pwa::FetchResponse {
                                status: cached.response.status,
                                headers: cached.response.headers.clone(),
                                body: cached.response.body.clone(),
                            })
                        } else {
                            Err(crate::pwa::cache::CacheError::CacheNotFound(request.url.clone()))
                        }
                    }
                }
            }
            
            CacheStrategy::NetworkOnly => {
                self.fetch_from_network(request).await
            }
            
            CacheStrategy::CacheOnly => {
                if let Some(cached) = cache.match_url(&request.url).await {
                    Ok(crate::pwa::FetchResponse {
                        status: cached.response.status,
                        headers: cached.response.headers.clone(),
                        body: cached.response.body.clone(),
                    })
                } else {
                    Err(crate::pwa::cache::CacheError::CacheNotFound(request.url.clone()))
                }
            }
            
            CacheStrategy::StaleWhileRevalidate => {
                if let Some(cached) = cache.match_url(&request.url).await {
                    tokio::spawn(async move {
                        let _ = self.fetch_from_network(request).await;
                    });
                    
                    Ok(crate::pwa::FetchResponse {
                        status: cached.response.status,
                        headers: cached.response.headers.clone(),
                        body: cached.response.body.clone(),
                    })
                } else {
                    self.fetch_from_network(request).await
                }
            }
        }
    }

    async fn fetch_from_network(&self, request: &crate::pwa::FetchRequest) -> Result<crate::pwa::FetchResponse, crate::pwa::cache::CacheError> {
        let client = reqwest::Client::new();
        
        let mut req_builder = match request.method.as_str() {
            "GET" => client.get(&request.url),
            "POST" => client.post(&request.url),
            "PUT" => client.put(&request.url),
            "DELETE" => client.delete(&request.url),
            _ => client.get(&request.url),
        };

        for (key, value) in &request.headers {
            req_builder = req_builder.header(key, value);
        }

        if let Some(body) = &request.body {
            req_builder = req_builder.body(body.clone());
        }

        let response = req_builder.send().await
            .map_err(|e| crate::pwa::cache::CacheError::IoError(e.to_string()))?;

        let status = response.status().as_u16();
        let headers = response.headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        
        let body = response.bytes().await
            .map_err(|e| crate::pwa::cache::CacheError::IoError(e.to_string()))?
            .to_vec();

        Ok(crate::pwa::FetchResponse {
            status,
            headers,
            body,
        })
    }
}

impl Default for CacheStrategy {
    fn default() -> Self {
        CacheStrategy::CacheFirst
    }
}