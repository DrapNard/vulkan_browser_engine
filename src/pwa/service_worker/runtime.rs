use super::{ServiceWorker, ServiceWorkerError};
use crate::js_engine::JsEngine;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ServiceWorkerRuntime {
    js_engine: Arc<RwLock<JsEngine>>,
    event_handlers: Arc<RwLock<HashMap<String, EventHandlers>>>,
}

struct EventHandlers {
    install: Option<String>,
    activate: Option<String>,
    fetch: Option<String>,
    message: Option<String>,
    sync: Option<String>,
    push: Option<String>,
}

impl ServiceWorkerRuntime {
    pub fn new() -> Self {
        Self {
            js_engine: Arc::new(RwLock::new(JsEngine::new().expect("Failed to create JS engine"))),
            event_handlers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn install_worker(&mut self, script_url: &str, scope: &str) -> Result<(), ServiceWorkerError> {
        let script_content = self.fetch_script(script_url).await?;
        
        let mut js_engine = self.js_engine.write().await;
        
        self.setup_service_worker_globals(&mut js_engine, scope).await?;
        
        js_engine.execute(&script_content).await
            .map_err(|e| ServiceWorkerError::ScriptError(e.to_string()))?;

        self.extract_event_handlers(&mut js_engine, script_url).await?;

        self.fire_install_event(&mut js_engine, script_url).await?;

        log::info!("Service Worker installed: {}", script_url);
        Ok(())
    }

    pub async fn activate_worker(&self, worker_id: &str) -> Result<(), ServiceWorkerError> {
        let mut js_engine = self.js_engine.write().await;
        
        self.fire_activate_event(&mut js_engine, worker_id).await?;

        log::info!("Service Worker activated: {}", worker_id);
        Ok(())
    }

    pub async fn terminate_worker(&self, worker_id: &str) -> Result<(), ServiceWorkerError> {
        let mut event_handlers = self.event_handlers.write().await;
        event_handlers.remove(worker_id);

        log::info!("Service Worker terminated: {}", worker_id);
        Ok(())
    }

    pub async fn update_worker(&self, script_url: &str, scope: &str) -> Result<(), ServiceWorkerError> {
        let script_content = self.fetch_script(script_url).await?;
        
        let mut js_engine = self.js_engine.write().await;
        
        js_engine.execute(&script_content).await
            .map_err(|e| ServiceWorkerError::ScriptError(e.to_string()))?;

        log::info!("Service Worker updated: {}", script_url);
        Ok(())
    }

    pub async fn handle_fetch_event(&self, worker: &ServiceWorker, request: &crate::pwa::FetchRequest) -> Result<Option<crate::pwa::FetchResponse>, ServiceWorkerError> {
        let event_handlers = self.event_handlers.read().await;
        if let Some(handlers) = event_handlers.get(&worker.id) {
            if let Some(_fetch_handler) = &handlers.fetch {
                let mut js_engine = self.js_engine.write().await;
                
                let fetch_event_script = format!(
                    r#"
                    if (typeof self.onfetch === 'function') {{
                        const event = new FetchEvent('{}', {{
                            request: new Request('{}', {{
                                method: '{}',
                                headers: {}
                            }})
                        }});
                        self.onfetch(event);
                    }}
                    "#,
                    request.url,
                    request.url,
                    request.method,
                    serde_json::to_string(&request.headers).unwrap_or_default()
                );

                js_engine.execute(&fetch_event_script).await
                    .map_err(|e| ServiceWorkerError::ExecutionError(e.to_string()))?;

                return Ok(Some(crate::pwa::FetchResponse {
                    status: 200,
                    headers: HashMap::new(),
                    body: b"Service Worker handled".to_vec(),
                }));
            }
        }
        Ok(None)
    }

    async fn setup_service_worker_globals(&self, js_engine: &mut JsEngine, scope: &str) -> Result<(), ServiceWorkerError> {
        let globals_script = format!(
            r#"
            const self = globalThis;
            self.registration = {{
                scope: '{}',
                update: () => Promise.resolve(),
                unregister: () => Promise.resolve(true)
            }};
            
            self.caches = {{
                open: (name) => Promise.resolve({{
                    match: (request) => Promise.resolve(undefined),
                    add: (request) => Promise.resolve(),
                    addAll: (requests) => Promise.resolve(),
                    put: (request, response) => Promise.resolve(),
                    delete: (request) => Promise.resolve(false)
                }}),
                match: (request) => Promise.resolve(undefined),
                has: (name) => Promise.resolve(false),
                delete: (name) => Promise.resolve(false),
                keys: () => Promise.resolve([])
            }};
            
            class ExtendableEvent {{
                constructor(type) {{
                    this.type = type;
                    this.promises = [];
                }}
                
                waitUntil(promise) {{
                    this.promises.push(promise);
                }}
            }}
            
            class FetchEvent extends ExtendableEvent {{
                constructor(type, init) {{
                    super(type);
                    this.request = init.request;
                    this.clientId = init.clientId || '';
                    this.handled = false;
                }}
                
                respondWith(response) {{
                    this.handled = true;
                    return response;
                }}
            }}
            
            class InstallEvent extends ExtendableEvent {{
                constructor() {{
                    super('install');
                }}
            }}
            
            class ActivateEvent extends ExtendableEvent {{
                constructor() {{
                    super('activate');
                }}
            }}
            
            self.ExtendableEvent = ExtendableEvent;
            self.FetchEvent = FetchEvent;
            self.InstallEvent = InstallEvent;
            self.ActivateEvent = ActivateEvent;
            
            self.addEventListener = function(type, listener) {{
                self['on' + type] = listener;
            }};
            
            self.skipWaiting = () => Promise.resolve();
            self.clients = {{
                claim: () => Promise.resolve(),
                matchAll: () => Promise.resolve([])
            }};
            "#,
            scope
        );

        js_engine.execute(&globals_script).await
            .map_err(|e| ServiceWorkerError::ScriptError(e.to_string()))?;

        Ok(())
    }

    async fn extract_event_handlers(&self, js_engine: &mut JsEngine, worker_id: &str) -> Result<(), ServiceWorkerError> {
        let handlers = EventHandlers {
            install: None,
            activate: None,
            fetch: None,
            message: None,
            sync: None,
            push: None,
        };

        let mut event_handlers = self.event_handlers.write().await;
        event_handlers.insert(worker_id.to_string(), handlers);

        Ok(())
    }

    async fn fire_install_event(&self, js_engine: &mut JsEngine, worker_id: &str) -> Result<(), ServiceWorkerError> {
        let install_script = r#"
            if (typeof self.oninstall === 'function') {
                const event = new InstallEvent();
                self.oninstall(event);
                Promise.all(event.promises).then(() => {
                    console.log('Install event completed');
                }).catch((error) => {
                    console.error('Install event failed:', error);
                });
            }
        "#;

        js_engine.execute(install_script).await
            .map_err(|e| ServiceWorkerError::ExecutionError(e.to_string()))?;

        Ok(())
    }

    async fn fire_activate_event(&self, js_engine: &mut JsEngine, worker_id: &str) -> Result<(), ServiceWorkerError> {
        let activate_script = r#"
            if (typeof self.onactivate === 'function') {
                const event = new ActivateEvent();
                self.onactivate(event);
                Promise.all(event.promises).then(() => {
                    console.log('Activate event completed');
                }).catch((error) => {
                    console.error('Activate event failed:', error);
                });
            }
        "#;

        js_engine.execute(activate_script).await
            .map_err(|e| ServiceWorkerError::ExecutionError(e.to_string()))?;

        Ok(())
    }

    async fn fetch_script(&self, script_url: &str) -> Result<String, ServiceWorkerError> {
        let response = reqwest::get(script_url).await
            .map_err(|e| ServiceWorkerError::NetworkError(e.to_string()))?;

        response.text().await
            .map_err(|e| ServiceWorkerError::NetworkError(e.to_string()))
    }
}

impl Default for ServiceWorkerRuntime {
    fn default() -> Self {
        Self::new()
    }
}