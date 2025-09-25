use super::ServiceWorkerError;
use crate::js_engine::JSRuntime as JsEngine;
use crate::BrowserConfig;
use base64::{engine::general_purpose, Engine as _};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::timeout;
use tracing::{error, info, warn};

#[derive(Debug, Clone)]
pub struct ServiceWorkerConfig {
    pub max_workers: usize,
    pub execution_timeout: Duration,
    pub script_timeout: Duration,
    pub max_script_size: usize,
    pub max_idle_time: Duration,
    pub enable_https_only: bool,
}

pub struct ServiceWorkerRuntime {
    workers: Arc<RwLock<HashMap<String, WorkerInstance>>>,
    http_client: reqwest::Client,
    config: ServiceWorkerConfig,
}

struct WorkerInstance {
    id: String,
    js_engine: JsEngine,
    scope: String,
    script_url: String,
    state: WorkerState,
    event_handlers: EventHandlerRegistry,
    last_activity: Instant,
    execution_stats: ExecutionStats,
}

#[derive(Debug, Clone, PartialEq)]
enum WorkerState {
    Installing,
    Installed,
    Activating,
    Activated,
    Redundant,
}

#[derive(Default)]
struct EventHandlerRegistry {
    install: bool,
    activate: bool,
    fetch: bool,
    message: bool,
    sync: bool,
    push: bool,
    notificationclick: bool,
}

#[derive(Debug, Default, Clone)]
pub struct ExecutionStats {
    total_executions: u64,
    total_duration: Duration,
    last_execution_time: Option<Duration>,
    error_count: u64,
    success_rate: f64,
}

impl ServiceWorkerRuntime {
    pub async fn new() -> Result<Self, ServiceWorkerError> {
        Self::with_config(ServiceWorkerConfig::default()).await
    }

    pub async fn with_config(config: ServiceWorkerConfig) -> Result<Self, ServiceWorkerError> {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(5))
            .user_agent("VulkanBrowser-ServiceWorker/1.0")
            .pool_max_idle_per_host(4)
            .build()
            .map_err(|e| ServiceWorkerError::NetworkError(e.to_string()))?;

        Ok(Self {
            workers: Arc::new(RwLock::new(HashMap::new())),
            http_client,
            config,
        })
    }

    fn create_service_worker_js_config() -> BrowserConfig {
        BrowserConfig::default()
    }

    pub async fn install_worker(
        &self,
        script_url: &str,
        scope: &str,
    ) -> Result<String, ServiceWorkerError> {
        let worker_id = self.generate_worker_id(script_url, scope);

        if self.workers.read().await.len() >= self.config.max_workers {
            return Err(ServiceWorkerError::ExecutionError(
                "Maximum workers reached".to_string(),
            ));
        }

        let script_content = self.fetch_script_with_validation(script_url).await?;

        let js_config = Self::create_service_worker_js_config();

        let mut js_engine = JsEngine::new(&js_config)
            .await
            .map_err(|e| ServiceWorkerError::ScriptError(e.to_string()))?;

        self.setup_service_worker_environment(&mut js_engine, scope, &worker_id)
            .await?;

        self.execute_script_safely(&mut js_engine, &script_content, "worker_installation")
            .await?;

        let event_handlers = self.extract_event_handlers(&mut js_engine).await?;

        let worker_instance = WorkerInstance {
            id: worker_id.clone(),
            js_engine,
            scope: scope.to_string(),
            script_url: script_url.to_string(),
            state: WorkerState::Installing,
            event_handlers,
            last_activity: Instant::now(),
            execution_stats: ExecutionStats::default(),
        };

        let mut workers = self.workers.write().await;
        workers.insert(worker_id.clone(), worker_instance);
        drop(workers);

        self.fire_install_event(&worker_id).await?;

        info!("Service Worker installed successfully: {}", script_url);
        Ok(worker_id)
    }

    pub async fn activate_worker(&self, worker_id: &str) -> Result<(), ServiceWorkerError> {
        let mut workers = self.workers.write().await;
        let worker = workers
            .get_mut(worker_id)
            .ok_or_else(|| ServiceWorkerError::WorkerNotFound(worker_id.to_string()))?;

        if worker.state != WorkerState::Installed {
            return Err(ServiceWorkerError::ExecutionError(format!(
                "Worker {} cannot be activated from state {:?}",
                worker_id, worker.state
            )));
        }

        worker.state = WorkerState::Activating;
        drop(workers);

        self.fire_activate_event(worker_id).await?;

        let mut workers = self.workers.write().await;
        if let Some(worker) = workers.get_mut(worker_id) {
            worker.state = WorkerState::Activated;
        }

        info!("Service Worker activated: {}", worker_id);
        Ok(())
    }

    pub async fn terminate_worker(&self, worker_id: &str) -> Result<(), ServiceWorkerError> {
        let mut workers = self.workers.write().await;
        if let Some(mut worker) = workers.remove(worker_id) {
            worker.state = WorkerState::Redundant;
            info!("Service Worker terminated: {}", worker_id);
            Ok(())
        } else {
            Err(ServiceWorkerError::WorkerNotFound(worker_id.to_string()))
        }
    }

    pub async fn handle_fetch_event(
        &self,
        worker_id: &str,
        request: &crate::pwa::FetchRequest,
    ) -> Result<Option<crate::pwa::FetchResponse>, ServiceWorkerError> {
        let start_time = Instant::now();

        let mut workers = self.workers.write().await;
        let worker = workers
            .get_mut(worker_id)
            .ok_or_else(|| ServiceWorkerError::WorkerNotFound(worker_id.to_string()))?;

        if worker.state != WorkerState::Activated || !worker.event_handlers.fetch {
            return Ok(None);
        }

        worker.last_activity = Instant::now();

        let fetch_event_script = self.create_fetch_event_script(request)?;

        match self
            .execute_with_timeout(
                &mut worker.js_engine,
                &fetch_event_script,
                self.config.execution_timeout,
            )
            .await
        {
            Ok(result) => {
                let duration = start_time.elapsed();
                self.update_execution_stats(&mut worker.execution_stats, duration, true);

                if let Some(response_data) = result {
                    self.parse_fetch_response(response_data)
                } else {
                    Ok(Some(self.create_default_response()))
                }
            }
            Err(e) => {
                let duration = start_time.elapsed();
                self.update_execution_stats(&mut worker.execution_stats, duration, false);
                error!(
                    "Fetch event execution failed for worker {}: {}",
                    worker_id, e
                );
                Err(e)
            }
        }
    }

    pub async fn get_worker_stats(
        &self,
        worker_id: &str,
    ) -> Result<ExecutionStats, ServiceWorkerError> {
        let workers = self.workers.read().await;
        let worker = workers
            .get(worker_id)
            .ok_or_else(|| ServiceWorkerError::WorkerNotFound(worker_id.to_string()))?;

        Ok(worker.execution_stats.clone())
    }

    pub async fn list_active_workers(&self) -> Vec<String> {
        let workers = self.workers.read().await;
        workers
            .values()
            .filter(|w| matches!(w.state, WorkerState::Activated))
            .map(|w| w.id.clone())
            .collect()
    }

    pub async fn cleanup_inactive_workers(&self) -> usize {
        let mut workers = self.workers.write().await;
        let now = Instant::now();
        let mut removed = 0;

        workers.retain(|id, worker| {
            if worker.state == WorkerState::Redundant
                || (now.duration_since(worker.last_activity) > self.config.max_idle_time)
            {
                warn!("Cleaning up inactive worker: {}", id);
                removed += 1;
                false
            } else {
                true
            }
        });

        removed
    }

    async fn setup_service_worker_environment(
        &self,
        js_engine: &mut JsEngine,
        scope: &str,
        worker_id: &str,
    ) -> Result<(), ServiceWorkerError> {
        let globals_script = format!(
            r#"
            const self = globalThis;
            const __WORKER_ID__ = '{}';
            const __WORKER_SCOPE__ = '{}';
            
            self.registration = {{
                scope: __WORKER_SCOPE__,
                active: null,
                installing: null,
                waiting: null,
                addEventListener: () => {{}},
                update: () => Promise.resolve(),
                unregister: () => Promise.resolve(true)
            }};
            
            self.caches = {{
                open: (name) => Promise.resolve(new Cache()),
                match: (request) => Promise.resolve(undefined),
                has: (name) => Promise.resolve(false),
                delete: (name) => Promise.resolve(false),
                keys: () => Promise.resolve([])
            }};
            
            class Cache {{
                match(request) {{ return Promise.resolve(undefined); }}
                add(request) {{ return Promise.resolve(); }}
                addAll(requests) {{ return Promise.resolve(); }}
                put(request, response) {{ return Promise.resolve(); }}
                delete(request) {{ return Promise.resolve(false); }}
                keys() {{ return Promise.resolve([]); }}
            }}
            
            class ExtendableEvent {{
                constructor(type, eventInitDict = {{}}) {{
                    this.type = type;
                    this.cancelable = eventInitDict.cancelable || false;
                    this.promises = [];
                    this.defaultPrevented = false;
                }}
                
                waitUntil(promise) {{
                    if (!(promise instanceof Promise)) {{
                        throw new TypeError('Argument must be a Promise');
                    }}
                    this.promises.push(promise);
                }}
                
                preventDefault() {{
                    if (this.cancelable) {{
                        this.defaultPrevented = true;
                    }}
                }}
            }}
            
            class FetchEvent extends ExtendableEvent {{
                constructor(type, eventInitDict) {{
                    super(type, eventInitDict);
                    this.request = eventInitDict.request;
                    this.clientId = eventInitDict.clientId || '';
                    this.resultingClientId = eventInitDict.resultingClientId || '';
                    this.handled = false;
                    this.response = null;
                }}
                
                respondWith(response) {{
                    if (this.handled) {{
                        throw new Error('Event already handled');
                    }}
                    this.handled = true;
                    this.response = Promise.resolve(response);
                }}
            }}
            
            class InstallEvent extends ExtendableEvent {{
                constructor() {{ super('install'); }}
            }}
            
            class ActivateEvent extends ExtendableEvent {{
                constructor() {{ super('activate'); }}
            }}
            
            self.ExtendableEvent = ExtendableEvent;
            self.FetchEvent = FetchEvent;
            self.InstallEvent = InstallEvent;
            self.ActivateEvent = ActivateEvent;
            
            const eventListeners = new Map();
            
            self.addEventListener = function(type, listener, options = {{}}) {{
                if (!eventListeners.has(type)) {{
                    eventListeners.set(type, []);
                }}
                eventListeners.get(type).push({{ listener, options }});
                self['on' + type] = listener;
            }};
            
            self.removeEventListener = function(type, listener) {{
                if (eventListeners.has(type)) {{
                    const listeners = eventListeners.get(type);
                    const index = listeners.findIndex(l => l.listener === listener);
                    if (index !== -1) {{
                        listeners.splice(index, 1);
                    }}
                }}
            }};
            
            self.skipWaiting = () => Promise.resolve();
            
            self.clients = {{
                claim: () => Promise.resolve(),
                matchAll: (options = {{}}) => Promise.resolve([]),
                openWindow: (url) => Promise.resolve(null),
                get: (id) => Promise.resolve(null)
            }};
            
            self.importScripts = function() {{
                throw new Error('importScripts not supported in this environment');
            }};
            "#,
            worker_id, scope
        );

        self.execute_script_safely(js_engine, &globals_script, "globals_setup")
            .await?;
        Ok(())
    }

    async fn execute_script_safely(
        &self,
        js_engine: &mut JsEngine,
        script: &str,
        context: &str,
    ) -> Result<(), ServiceWorkerError> {
        timeout(self.config.script_timeout, js_engine.execute(script))
            .await
            .map_err(|_| ServiceWorkerError::ExecutionError(format!("Timeout in {}", context)))?
            .map_err(|e| ServiceWorkerError::ScriptError(format!("{}: {}", context, e)))?;

        Ok(())
    }

    async fn execute_with_timeout(
        &self,
        js_engine: &mut JsEngine,
        script: &str,
        timeout_duration: Duration,
    ) -> Result<Option<Value>, ServiceWorkerError> {
        timeout(timeout_duration, js_engine.execute(script))
            .await
            .map_err(|_| {
                ServiceWorkerError::ExecutionError("Script execution timeout".to_string())
            })?
            .map_err(|e| ServiceWorkerError::ExecutionError(e.to_string()))
            .map(Some)
    }

    async fn extract_event_handlers(
        &self,
        js_engine: &mut JsEngine,
    ) -> Result<EventHandlerRegistry, ServiceWorkerError> {
        let check_script = r#"
            JSON.stringify({
                install: typeof self.oninstall === 'function',
                activate: typeof self.onactivate === 'function',
                fetch: typeof self.onfetch === 'function',
                message: typeof self.onmessage === 'function',
                sync: typeof self.onsync === 'function',
                push: typeof self.onpush === 'function',
                notificationclick: typeof self.onnotificationclick === 'function'
            })
        "#;

        let result = self
            .execute_with_timeout(js_engine, check_script, Duration::from_secs(2))
            .await?;

        if let Some(Value::String(json_str)) = result {
            let handlers: serde_json::Value = serde_json::from_str(&json_str)
                .map_err(|e| ServiceWorkerError::ScriptError(e.to_string()))?;

            Ok(EventHandlerRegistry {
                install: handlers["install"].as_bool().unwrap_or(false),
                activate: handlers["activate"].as_bool().unwrap_or(false),
                fetch: handlers["fetch"].as_bool().unwrap_or(false),
                message: handlers["message"].as_bool().unwrap_or(false),
                sync: handlers["sync"].as_bool().unwrap_or(false),
                push: handlers["push"].as_bool().unwrap_or(false),
                notificationclick: handlers["notificationclick"].as_bool().unwrap_or(false),
            })
        } else {
            Ok(EventHandlerRegistry::default())
        }
    }

    async fn fire_install_event(&self, worker_id: &str) -> Result<(), ServiceWorkerError> {
        let install_script = r#"
            (async function() {
                if (typeof self.oninstall === 'function') {
                    const event = new InstallEvent();
                    try {
                        await self.oninstall(event);
                        if (event.promises.length > 0) {
                            await Promise.all(event.promises);
                        }
                        return { success: true };
                    } catch (error) {
                        return { success: false, error: error.message };
                    }
                }
                return { success: true };
            })()
        "#;

        let mut workers = self.workers.write().await;
        let worker = workers
            .get_mut(worker_id)
            .ok_or_else(|| ServiceWorkerError::WorkerNotFound(worker_id.to_string()))?;

        self.execute_with_timeout(
            &mut worker.js_engine,
            install_script,
            Duration::from_secs(30),
        )
        .await?;
        worker.state = WorkerState::Installed;

        Ok(())
    }

    async fn fire_activate_event(&self, worker_id: &str) -> Result<(), ServiceWorkerError> {
        let activate_script = r#"
            (async function() {
                if (typeof self.onactivate === 'function') {
                    const event = new ActivateEvent();
                    try {
                        await self.onactivate(event);
                        if (event.promises.length > 0) {
                            await Promise.all(event.promises);
                        }
                        return { success: true };
                    } catch (error) {
                        return { success: false, error: error.message };
                    }
                }
                return { success: true };
            })()
        "#;

        let mut workers = self.workers.write().await;
        let worker = workers
            .get_mut(worker_id)
            .ok_or_else(|| ServiceWorkerError::WorkerNotFound(worker_id.to_string()))?;

        self.execute_with_timeout(
            &mut worker.js_engine,
            activate_script,
            Duration::from_secs(30),
        )
        .await?;

        Ok(())
    }

    fn create_fetch_event_script(
        &self,
        request: &crate::pwa::FetchRequest,
    ) -> Result<String, ServiceWorkerError> {
        let headers_json = serde_json::to_string(&request.headers)
            .map_err(|e| ServiceWorkerError::ScriptError(e.to_string()))?;

        let body_json = if let Some(body) = &request.body {
            format!("\"{}\"", general_purpose::STANDARD.encode(body))
        } else {
            "null".to_string()
        };

        Ok(format!(
            r#"
            (async function() {{
                if (typeof self.onfetch === 'function') {{
                    const request = {{
                        url: '{}',
                        method: '{}',
                        headers: {},
                        body: {}
                    }};
                    
                    const event = new FetchEvent('fetch', {{ request }});
                    
                    try {{
                        await self.onfetch(event);
                        
                        if (event.handled && event.response) {{
                            const response = await event.response;
                            return {{
                                handled: true,
                                status: response.status || 200,
                                headers: response.headers || {{}},
                                body: response.body || ''
                            }};
                        }}
                        
                        return {{ handled: false }};
                    }} catch (error) {{
                        return {{ 
                            handled: false, 
                            error: error.message 
                        }};
                    }}
                }}
                return {{ handled: false }};
            }})()
            "#,
            request.url, request.method, headers_json, body_json
        ))
    }

    fn parse_fetch_response(
        &self,
        response_data: Value,
    ) -> Result<Option<crate::pwa::FetchResponse>, ServiceWorkerError> {
        if let Some(obj) = response_data.as_object() {
            if obj
                .get("handled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                let status = obj.get("status").and_then(|v| v.as_u64()).unwrap_or(200) as u16;
                let headers = obj
                    .get("headers")
                    .and_then(|v| v.as_object())
                    .map(|h| {
                        h.iter()
                            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let body = obj
                    .get("body")
                    .and_then(|v| v.as_str())
                    .map(|s| s.as_bytes().to_vec())
                    .unwrap_or_default();

                return Ok(Some(crate::pwa::FetchResponse {
                    status,
                    headers,
                    body,
                }));
            }
        }
        Ok(None)
    }

    fn create_default_response(&self) -> crate::pwa::FetchResponse {
        crate::pwa::FetchResponse {
            status: 200,
            headers: HashMap::new(),
            body: b"Service Worker handled request".to_vec(),
        }
    }

    async fn fetch_script_with_validation(
        &self,
        script_url: &str,
    ) -> Result<String, ServiceWorkerError> {
        if self.config.enable_https_only
            && !script_url.starts_with("https://")
            && !script_url.starts_with("http://localhost")
        {
            return Err(ServiceWorkerError::NetworkError(
                "Service Worker scripts must be served over HTTPS".to_string(),
            ));
        }

        let response = timeout(
            Duration::from_secs(30),
            self.http_client.get(script_url).send(),
        )
        .await
        .map_err(|_| ServiceWorkerError::NetworkError("Network timeout".to_string()))?
        .map_err(|e| ServiceWorkerError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(ServiceWorkerError::NetworkError(format!(
                "HTTP {}: Failed to fetch script",
                response.status()
            )));
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if !content_type.contains("javascript") && !content_type.contains("text/") {
            return Err(ServiceWorkerError::NetworkError(
                "Invalid content type for Service Worker script".to_string(),
            ));
        }

        let script_content = response
            .text()
            .await
            .map_err(|e| ServiceWorkerError::NetworkError(e.to_string()))?;

        if script_content.len() > self.config.max_script_size {
            return Err(ServiceWorkerError::NetworkError(format!(
                "Service Worker script too large (max {}MB)",
                self.config.max_script_size / (1024 * 1024)
            )));
        }

        Ok(script_content)
    }

    fn generate_worker_id(&self, script_url: &str, scope: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        script_url.hash(&mut hasher);
        scope.hash(&mut hasher);
        format!("sw_{:x}", hasher.finish())
    }

    fn update_execution_stats(
        &self,
        stats: &mut ExecutionStats,
        duration: Duration,
        success: bool,
    ) {
        stats.total_executions += 1;
        stats.total_duration += duration;
        stats.last_execution_time = Some(duration);

        if !success {
            stats.error_count += 1;
        }

        stats.success_rate = ((stats.total_executions - stats.error_count) as f64
            / stats.total_executions as f64)
            * 100.0;
    }
}

impl Default for ServiceWorkerConfig {
    fn default() -> Self {
        Self {
            max_workers: 10,
            execution_timeout: Duration::from_secs(5),
            script_timeout: Duration::from_secs(10),
            max_script_size: 5 * 1024 * 1024,
            max_idle_time: Duration::from_secs(300),
            enable_https_only: true,
        }
    }
}
