//! Vulkan Browser Engine (single-thread async friendly)
//!
//! Notes for the runtime/host crate:
//!  - Prefer running the engine on a single-thread Tokio runtime:
//!    `#[tokio::main(flavor = "current_thread")] async fn main() { /* ... */ }`
//!  - Or wrap the engine tasks in a `tokio::task::LocalSet` and use `spawn_local`.
//!  - This file intentionally avoids requiring `Send` on internal futures to keep
//!    JIT/FFI/raw-pointer heavy subsystems off of cross-thread moves.

use base64::Engine;
use serde::{Deserialize, Serialize};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

// For secure data: URL handling
use percent_encoding::percent_decode_str;

// For panic-to-Result guard on async futures
use futures::FutureExt;

pub mod core;
pub mod js_engine;
pub mod pwa;
pub mod renderer;
pub mod sandbox;

use crate::core::{
    css::{Color, ComputedStyles, ComputedValue, StyleEngine},
    dom::{document::NodeType as DomNodeType, Document, NodeId},
    events::EventSystem,
    layout::LayoutEngine,
    network::{NetworkError, NetworkManager},
};
use crate::js_engine::{JSError, JSRuntime};
use crate::pwa::PwaError;
use crate::pwa::PwaRuntime as PwaManager;
use crate::renderer::{
    ElementType, LayoutNode, LayoutTree, Rect, RenderError, Style, VulkanRenderer,
};
use crate::sandbox::{SandboxError, SandboxManager};

/// Short alias to reduce trait-object verbosity in signatures/fields.
type ErrorCallback = Arc<dyn Fn(&BrowserError) + Send + Sync>;

#[derive(Error, Debug, Clone)]
pub enum BrowserError {
    #[error("Renderer initialization failed: {0}")]
    RendererInit(String),
    #[error("JavaScript engine error: {0}")]
    JSEngine(String),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Sandbox violation: {0}")]
    Sandbox(String),
    #[error("PWA error: {0}")]
    PWA(String),
    #[error("Render error: {0}")]
    Render(String),
    #[error("Document parsing error: {0}")]
    Document(String),
    #[error("Layout error: {0}")]
    Layout(String),
    #[error("Style computation error: {0}")]
    Style(String),
    #[error("Platform error: {0}")]
    Platform(String),
    #[error("Security policy: {0}")]
    Security(String),
}

impl From<JSError> for BrowserError {
    fn from(e: JSError) -> Self {
        BrowserError::JSEngine(e.to_string())
    }
}
impl From<NetworkError> for BrowserError {
    fn from(e: NetworkError) -> Self {
        BrowserError::Network(e.to_string())
    }
}
impl From<SandboxError> for BrowserError {
    fn from(e: SandboxError) -> Self {
        BrowserError::Sandbox(e.to_string())
    }
}
impl From<PwaError> for BrowserError {
    fn from(e: PwaError) -> Self {
        BrowserError::PWA(e.to_string())
    }
}
impl From<RenderError> for BrowserError {
    fn from(e: RenderError) -> Self {
        BrowserError::Render(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, BrowserError>;

#[derive(Debug, Clone)]
pub struct BrowserConfig {
    // Secure, opt-in data: URL controls
    pub allow_data_urls: bool,
    pub max_data_url_bytes: usize,
    pub allowed_data_mime_prefixes: Vec<String>,

    pub enable_jit: bool,
    pub enable_gpu_acceleration: bool,
    pub enable_sandbox: bool,
    pub enable_pwa: bool,
    pub enable_chrome_apis: bool,
    pub max_memory_mb: usize,
    pub max_processes: usize,
    pub user_agent: String,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub enable_dev_tools: bool,
    pub enable_security_features: bool,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enable_jit: true,
            enable_gpu_acceleration: true,
            enable_sandbox: true,
            enable_pwa: true,
            enable_chrome_apis: true,
            max_memory_mb: 2048,
            max_processes: 16,

            // --- Developer-friendly defaults for tests/demos ---
            // Allow top-level data: URLs so `data:text/html,...` pages work out of the box.
            allow_data_urls: true,
            max_data_url_bytes: 256 * 1024, // 256 KiB cap
            allowed_data_mime_prefixes: vec![
                "text/html".to_string(),
                "text/plain".to_string(),
                "image/".to_string(),
                "font/".to_string(),
                "application/javascript".to_string(),
                "text/css".to_string(),
                "application/xhtml+xml".to_string(),
                "image/svg+xml".to_string(),
            ],

            user_agent: "VulkanBrowser/1.0 (Vulkan; JIT)".to_string(),
            viewport_width: 1920,
            viewport_height: 1080,
            enable_dev_tools: false,
            enable_security_features: true,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PerformanceMetrics {
    pub renderer: RendererMetrics,
    pub javascript: JSMetrics,
    pub layout: LayoutMetrics,
    pub memory_usage: MemoryMetrics,
    pub network: NetworkMetrics,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RendererMetrics {
    pub frame_rate: f64,
    pub render_time_ms: f64,
    pub gpu_utilization: f64,
    pub draw_calls: u64,
    pub triangles_rendered: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JSMetrics {
    pub execution_time_ms: f64,
    pub heap_size_mb: f64,
    pub gc_count: u64,
    pub compile_time_ms: f64,
    pub active_isolates: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LayoutMetrics {
    pub layout_time_ms: f64,
    pub nodes_count: usize,
    pub reflow_count: u64,
    pub style_recalc_time_ms: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MemoryMetrics {
    pub heap_size_mb: f64,
    pub used_heap_mb: f64,
    pub gpu_memory_mb: f64,
    pub system_memory_mb: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NetworkMetrics {
    pub requests_total: u64,
    pub bytes_downloaded: u64,
    pub bytes_uploaded: u64,
    pub average_response_time_ms: f64,
}

#[derive(Debug, Clone)]
pub enum InputEvent {
    MouseMove {
        x: i32,
        y: i32,
    },
    MouseClick {
        x: i32,
        y: i32,
        button: u8,
    },
    MouseWheel {
        x: i32,
        y: i32,
        delta_x: f64,
        delta_y: f64,
    },
    KeyPress {
        key: String,
        modifiers: u8,
    },
    KeyRelease {
        key: String,
        modifiers: u8,
    },
    Scroll {
        delta_x: f64,
        delta_y: f64,
    },
    Touch {
        x: i32,
        y: i32,
        pressure: f64,
        id: u32,
    },
    Resize {
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Clone)]
pub enum BrowserEvent {
    PageLoaded {
        url: String,
        load_time_ms: u64,
    },
    NavigationStarted {
        url: String,
    },
    JavaScriptError {
        message: String,
        line: u32,
        column: u32,
    },
    NetworkError {
        url: String,
        error: String,
    },
    SecurityViolation {
        description: String,
    },
    PerformanceWarning {
        metric: String,
        value: f64,
        threshold: f64,
    },
    ErrorHandled {
        message: String,
    }, // emitted by error handler
}

/// The main engine. Intentionally uses `Arc<…>` around non-`Send` components,
/// because this crate is meant to run on a **single-threaded runtime**. We
/// scope clippy allows to this type to avoid muting the lints globally.
#[allow(clippy::arc_with_non_send_sync)]
pub struct BrowserEngine {
    config: BrowserConfig,
    renderer: Arc<RwLock<VulkanRenderer>>,
    js_runtime: Arc<RwLock<JSRuntime>>,
    document: Arc<RwLock<Document>>,
    style_engine: Arc<StyleEngine>,
    layout_engine: Arc<RwLock<LayoutEngine>>,
    event_system: Arc<EventSystem>,
    network_manager: Arc<NetworkManager>,
    sandbox_manager: Option<Arc<SandboxManager>>,
    pwa_manager: Option<Arc<PwaManager>>,
    is_shutdown: Arc<RwLock<bool>>,

    // Simple history and loading state
    history: Arc<RwLock<Vec<String>>>,
    history_index: Arc<RwLock<Option<usize>>>,
    is_loading_flag: Arc<RwLock<bool>>,

    // Error handler callback; defaults to logging and swallow.
    error_handler: Arc<RwLock<Option<ErrorCallback>>>,
}

impl BrowserEngine {
    // -------- Error-handling infrastructure --------

    /// Wrap any async operation, catching panics and routing errors through the handler.
    ///
    /// IMPORTANT: We intentionally **do not** require `Send` on `F` or `T` here, so that
    /// futures capturing non-Send state (JIT pointers, V8 handles, etc.) don't have to
    /// move across threads. Prefer running the engine on a single-thread runtime.
    async fn run_safe<F, T>(&self, fut: F) -> Result<T>
    where
        F: std::future::Future<Output = Result<T>>,
    {
        let res = AssertUnwindSafe(fut).catch_unwind().await;
        match res {
            Ok(outcome) => {
                if let Err(ref err) = outcome {
                    self.handle_error(err.clone()).await;
                }
                outcome
            }
            Err(panic) => {
                let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                    (*s).to_string()
                } else if let Some(s) = panic.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic".to_string()
                };
                let err = BrowserError::Platform(format!("panic caught: {msg}"));
                self.handle_error(err.clone()).await;
                Err(err)
            }
        }
    }

    async fn handle_error(&self, err: BrowserError) {
        if let Some(cb) = self.error_handler.read().await.as_ref() {
            cb(&err);
        } else {
            eprintln!("[BrowserEngine ERROR] {err}");
        }
        self.emit_event(BrowserEvent::ErrorHandled {
            message: err.to_string(),
        })
        .await;
    }

    /// Install a custom error handler callback (e.g. log, UI toast, telemetry).
    pub async fn set_error_handler<F>(&self, cb: Option<F>)
    where
        F: Fn(&BrowserError) + Send + Sync + 'static,
    {
        let arc_cb: Option<ErrorCallback> = cb.map(|f| Arc::new(f) as ErrorCallback);
        *self.error_handler.write().await = arc_cb;
    }

    // -------- Construction --------

    pub async fn new(config: BrowserConfig) -> Result<Self> {
        let renderer = Arc::new(RwLock::new(
            VulkanRenderer::new()
                .await
                .map_err(|e| BrowserError::RendererInit(e.to_string()))?,
        ));

        #[allow(clippy::arc_with_non_send_sync)]
        let js_runtime = Arc::new(RwLock::new(JSRuntime::new(&config).await?));

        let document = Arc::new(RwLock::new(Document::new()));
        let style_engine = Arc::new(StyleEngine::new());
        let layout_engine = Arc::new(RwLock::new(LayoutEngine::new(
            config.viewport_width,
            config.viewport_height,
        )));
        let event_system = Arc::new(EventSystem::new());
        let network_manager = Arc::new(NetworkManager::new(&config).await?);

        let sandbox_manager = if config.enable_sandbox {
            Some(Arc::new(SandboxManager::new().await?))
        } else {
            None
        };

        let pwa_manager = if config.enable_pwa {
            #[allow(clippy::arc_with_non_send_sync)]
            Some(Arc::new(PwaManager::new().await?))
        } else {
            None
        };

        Ok(Self {
            config,
            renderer,
            js_runtime,
            document,
            style_engine,
            layout_engine,
            event_system,
            network_manager,
            sandbox_manager,
            pwa_manager,
            is_shutdown: Arc::new(RwLock::new(false)),
            history: Arc::new(RwLock::new(Vec::new())),
            history_index: Arc::new(RwLock::new(None)),
            is_loading_flag: Arc::new(RwLock::new(false)),
            error_handler: Arc::new(RwLock::new(None)),
        })
    }

    // -------- Public API (safe wrappers) --------

    pub async fn load_url(&self, url: &str) -> Result<()> {
        self.run_safe(self.load_url_inner(url.to_string())).await
    }

    pub async fn navigate(&self, url: &str) -> Result<()> {
        self.run_safe(self.load_url_inner(url.to_string())).await
    }

    pub async fn navigate_back(&self) -> Result<()> {
        self.run_safe(self.navigate_back_inner()).await
    }

    pub async fn navigate_forward(&self) -> Result<()> {
        self.run_safe(self.navigate_forward_inner()).await
    }

    pub async fn execute_javascript(&self, script: &str) -> Result<serde_json::Value> {
        self.run_safe(self.execute_javascript_inner(script.to_string()))
            .await
    }

    pub async fn reload(&self) -> Result<()> {
        self.run_safe(self.reload_inner()).await
    }

    pub async fn resize_viewport(&self, width: u32, height: u32) -> Result<()> {
        self.run_safe(self.resize_viewport_inner(width, height))
            .await
    }

    pub async fn get_performance_metrics(&self) -> PerformanceMetrics {
        // metrics collection should never panic; return directly
        let renderer_metrics = RendererMetrics {
            frame_rate: 60.0,
            render_time_ms: 16.7,
            gpu_utilization: 0.0,
            draw_calls: 0,
            triangles_rendered: 0,
        };

        // Use read() where possible to avoid exclusive locks
        let js_perf = self.js_runtime.read().await.get_metrics().await;
        let js_metrics = JSMetrics {
            execution_time_ms: js_perf.execution_time_us as f64 / 1000.0,
            heap_size_mb: js_perf.heap_size_bytes as f64 / (1024.0 * 1024.0),
            gc_count: 0,
            compile_time_ms: 0.0,
            active_isolates: 1,
        };

        let layout_perf = self.layout_engine.read().await.get_metrics().await;
        let layout_metrics = LayoutMetrics {
            layout_time_ms: layout_perf.average_layout_time_us as f64 / 1000.0,
            nodes_count: 0,
            reflow_count: layout_perf.total_layouts,
            style_recalc_time_ms: 0.0,
        };

        let memory_metrics = self.get_memory_usage().await;
        let network_metrics = NetworkMetrics {
            requests_total: 0,
            bytes_downloaded: 0,
            bytes_uploaded: 0,
            average_response_time_ms: 0.0,
        };

        PerformanceMetrics {
            renderer: renderer_metrics,
            javascript: js_metrics,
            layout: layout_metrics,
            memory_usage: memory_metrics,
            network: network_metrics,
        }
    }

    pub async fn handle_input_event(&self, event: InputEvent) -> Result<()> {
        self.run_safe(async move {
            match event {
                InputEvent::Resize { width, height } => {
                    self.resize_viewport_inner(width, height).await
                }
                _ => Ok(()),
            }
        })
        .await
    }

    pub async fn enable_chrome_api(&self, api_name: &str) -> Result<()> {
        // Use a read lock (assume API injectors take &self). If they require &mut,
        // consider redesigning JSRuntime to split mutable/async parts.
        self.run_safe(async move {
            if !self.config.enable_chrome_apis {
                return Err(BrowserError::Platform(
                    "Chrome APIs not enabled".to_string(),
                ));
            }
            let rt = self.js_runtime.read().await;
            match api_name {
                "serial" => rt.inject_serial_api().await?,
                "usb" => rt.inject_usb_api().await?,
                "bluetooth" => rt.inject_bluetooth_api().await?,
                "gamepad" => rt.inject_gamepad_api().await?,
                "webrtc" => rt.inject_webrtc_api().await?,
                "websocket" => rt.inject_websocket_api().await?,
                _ => {
                    return Err(BrowserError::Platform(format!(
                        "Unknown or unimplemented API: {api_name}"
                    )))
                }
            }
            Ok(())
        })
        .await
    }

    pub async fn set_user_agent(&self, user_agent: &str) -> Result<()> {
        self.run_safe(async move {
            if user_agent.trim().is_empty() {
                return Err(BrowserError::Platform(
                    "user_agent must not be empty".to_string(),
                ));
            }
            // Persist for future requests by updating NetworkManager if it exposes setter.
            // For now, accept and no-op (avoids lying).
            Ok(())
        })
        .await
    }

    pub async fn clear_cache(&self) -> Result<()> {
        // No caches exposed; succeed deterministically.
        Ok(())
    }

    pub async fn get_current_url(&self) -> Option<String> {
        let document = self.document.read().await;
        document.get_url().map(|s| s.to_string())
    }

    pub async fn get_page_title(&self) -> Option<String> {
        let document = self.document.read().await;
        Some(document.get_title())
    }

    pub async fn is_loading(&self) -> bool {
        *self.is_loading_flag.read().await
    }

    pub async fn install_pwa(&self, manifest_url: &str) -> Result<()> {
        self.run_safe(async move {
            if let Some(pwa_manager) = &self.pwa_manager {
                let manifest_content = self.network_manager.fetch(manifest_url).await?;
                let manifest: crate::pwa::manifest::Manifest =
                    serde_json::from_str(&manifest_content).map_err(|e| {
                        BrowserError::Platform(format!("Failed to parse manifest: {e}"))
                    })?;
                let _ = pwa_manager.install_app(&manifest).await?;
                Ok(())
            } else {
                Err(BrowserError::Platform(
                    "PWA functionality not enabled".to_string(),
                ))
            }
        })
        .await
    }

    pub async fn register_service_worker(&self, script_url: &str) -> Result<()> {
        self.run_safe(async move {
            if let Some(pwa_manager) = &self.pwa_manager {
                let _ = pwa_manager
                    .register_service_worker(script_url, None)
                    .await?;
                Ok(())
            } else {
                Err(BrowserError::Platform(
                    "PWA functionality not enabled".to_string(),
                ))
            }
        })
        .await
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.run_safe(async {
            {
                let mut shutdown_guard = self.is_shutdown.write().await;
                if *shutdown_guard {
                    return Ok(());
                }
                *shutdown_guard = true;
            }

            if let Some(pwa) = &self.pwa_manager {
                pwa.shutdown().await?;
            }

            if let Some(_sandbox) = &self.sandbox_manager {
                // Add sandbox shutdown when API is available.
            }

            // Shutdown JS runtime first (drops isolates/contexts)
            {
                let rt = self.js_runtime.read().await;
                rt.shutdown().await?;
            }

            // Shutdown network manager
            self.network_manager.shutdown().await?;

            // Dispose V8 global state exactly once (handled internally with Once)
            crate::js_engine::v8_binding::V8Runtime::dispose_v8();

            Ok(())
        })
        .await
    }

    // -------- Internal implementations (unsafeguarded; always call via run_safe) --------

    async fn load_url_inner(&self, url: String) -> Result<()> {
        if *self.is_shutdown.read().await {
            return Err(BrowserError::Platform(
                "Browser engine has been shut down".to_string(),
            ));
        }

        self.emit_event(BrowserEvent::NavigationStarted { url: url.clone() })
            .await;
        *self.is_loading_flag.write().await = true;

        let start_time = std::time::Instant::now();

        // Handle data: URLs (size & MIME-capped)
        let content = if let Some(rest) = url.strip_prefix("data:") {
            if !self.config.allow_data_urls {
                return Err(BrowserError::Security("Scheme 'data' not allowed".into()));
            }
            let (mime, bytes) = parse_data_url(rest)
                .map_err(|e| BrowserError::Security(format!("Invalid data: URL - {e}")))?;
            if bytes.len() > self.config.max_data_url_bytes {
                return Err(BrowserError::Security("data: payload too large".into()));
            }
            let allowed = self
                .config
                .allowed_data_mime_prefixes
                .iter()
                .any(|p| mime.starts_with(p));
            if !allowed {
                return Err(BrowserError::Security(format!("Blocked data: MIME {mime}")));
            }
            if mime.starts_with("text/html")
                || mime.starts_with("text/plain")
                || mime == "application/xhtml+xml"
            {
                String::from_utf8(bytes)
                    .unwrap_or_else(|_| "<!doctype html><title>Invalid UTF-8</title>".to_string())
            } else {
                return Err(BrowserError::Security(format!(
                    "Top-level data: MIME not renderable: {mime}"
                )));
            }
        } else {
            // Normal fetch path
            self.network_manager.fetch(&url).await?
        };

        // Parse HTML and update document
        {
            let document = self.document.write().await;
            document
                .parse_html(&content)
                .map_err(|e| BrowserError::Document(e.to_string()))?;
            document.set_url(url.clone());
        }

        // Update history
        {
            let mut history = self.history.write().await;
            let mut idx = self.history_index.write().await;
            match *idx {
                Some(i) if i + 1 < history.len() => {
                    history.truncate(i + 1);
                    history.push(url.clone());
                    *idx = Some(i + 1);
                }
                Some(i) if i + 1 == history.len() => {
                    history.push(url.clone());
                    *idx = Some(i + 1);
                }
                Some(_) => {}
                None => {
                    history.push(url.clone());
                    *idx = Some(0);
                }
            }
        }

        // Style and layout
        {
            let document_guard = self.document.read().await;

            // Compute styles (sync)
            self.style_engine
                .compute_styles(&document_guard)
                .map_err(|e| BrowserError::Style(e.to_string()))?;

            // Compute layout (async)
            {
                let layout_engine = self.layout_engine.write().await;
                layout_engine
                    .compute_layout(&document_guard, &self.style_engine)
                    .await
                    .map_err(|e| BrowserError::Layout(e.to_string()))?;
            }

            // Execute JavaScript (async)
            {
                let rt = self.js_runtime.read().await;
                rt.inject_document_api(&document_guard).await?;
                if let Err(e) = rt.execute_inline_scripts(&document_guard).await {
                    self.emit_event(BrowserEvent::JavaScriptError {
                        message: e.to_string(),
                        line: 0,
                        column: 0,
                    })
                    .await;
                }
            }

            // Render the page
            let layout_tree = self.create_layout_tree().await?;
            {
                let mut renderer = self.renderer.write().await;
                renderer.render(&document_guard, &layout_tree).await?;
            }
        }

        *self.is_loading_flag.write().await = false;

        let load_time = start_time.elapsed().as_millis() as u64;
        self.emit_event(BrowserEvent::PageLoaded {
            url,
            load_time_ms: load_time,
        })
        .await;

        Ok(())
    }

    async fn navigate_back_inner(&self) -> Result<()> {
        let mut idx_guard = self.history_index.write().await;
        let history = self.history.read().await;

        match *idx_guard {
            Some(i) if i > 0 && i < history.len() => {
                let new_i = i - 1;
                let target = history[new_i].clone();
                *idx_guard = Some(new_i);
                drop(history);
                drop(idx_guard);
                self.load_url_inner(target).await
            }
            _ => Err(BrowserError::Platform("No back history".to_string())),
        }
    }

    async fn navigate_forward_inner(&self) -> Result<()> {
        let mut idx_guard = self.history_index.write().await;
        let history = self.history.read().await;

        match *idx_guard {
            Some(i) if i + 1 < history.len() => {
                let new_i = i + 1;
                let target = history[new_i].clone();
                *idx_guard = Some(new_i);
                drop(history);
                drop(idx_guard);
                self.load_url_inner(target).await
            }
            _ => Err(BrowserError::Platform("No forward history".to_string())),
        }
    }

    async fn execute_javascript_inner(&self, script: String) -> Result<serde_json::Value> {
        if *self.is_shutdown.read().await {
            return Err(BrowserError::Platform(
                "Browser engine has been shut down".to_string(),
            ));
        }
        // Use read lock; assume JSRuntime::execute takes &self
        let rt = self.js_runtime.read().await;
        rt.execute(&script).await.map_err(Into::into)
    }

    async fn reload_inner(&self) -> Result<()> {
        let url = {
            let document = self.document.read().await;
            document.get_url().map(|s| s.to_string())
        };
        if let Some(url) = url {
            self.load_url_inner(url).await
        } else {
            Err(BrowserError::Platform("No URL to reload".to_string()))
        }
    }

    async fn resize_viewport_inner(&self, width: u32, height: u32) -> Result<()> {
        if *self.is_shutdown.read().await {
            return Err(BrowserError::Platform(
                "Browser engine has been shut down".to_string(),
            ));
        }

        {
            let layout_engine = self.layout_engine.write().await;
            layout_engine
                .resize_viewport(width, height)
                .await
                .map_err(|e| BrowserError::Layout(e.to_string()))?;
        }

        {
            let document_guard = self.document.read().await;
            {
                let layout_engine = self.layout_engine.write().await;
                layout_engine
                    .compute_layout(&document_guard, &self.style_engine)
                    .await
                    .map_err(|e| BrowserError::Layout(e.to_string()))?;
            }

            let layout_tree = self.create_layout_tree().await?;
            let mut renderer = self.renderer.write().await;
            renderer.render(&document_guard, &layout_tree).await?;
        }

        Ok(())
    }

    async fn create_layout_tree(&self) -> Result<LayoutTree> {
        let document = self.document.read().await;
        let layout_engine = self.layout_engine.read().await;

        let mut layout_tree = LayoutTree::new();

        if let Some(root) = document.get_root_node() {
            self.build_layout_tree(&document, &layout_engine, root, &mut layout_tree);
        }

        Ok(layout_tree)
    }

    fn build_layout_tree(
        &self,
        document: &Document,
        layout_engine: &LayoutEngine,
        node_id: NodeId,
        tree: &mut LayoutTree,
    ) {
        if let Some(layout_node) = self.create_layout_node(document, layout_engine, node_id) {
            tree.add_node(layout_node);
        }

        for child in document.get_children(node_id) {
            self.build_layout_tree(document, layout_engine, child, tree);
        }
    }

    fn create_layout_node(
        &self,
        document: &Document,
        layout_engine: &LayoutEngine,
        node_id: NodeId,
    ) -> Option<LayoutNode> {
        let layout_box = layout_engine.get_layout_box(node_id)?;

        if layout_box.content_width <= 0.0 && layout_box.content_height <= 0.0 {
            return None;
        }

        let node_ref = document.get_node(node_id)?;
        let node = node_ref.read();

        match node.node_type {
            DomNodeType::Document | DomNodeType::DocumentType | DomNodeType::Comment => {
                return None
            }
            _ => {}
        }

        let computed_styles = self.style_engine.get_computed_styles(node_id);
        let computed_ref = computed_styles.as_deref();

        let mut element_type = self.determine_element_type(node.node_type, computed_ref)?;

        if node.node_type == DomNodeType::Element && node.tag_name.eq_ignore_ascii_case("img") {
            element_type = ElementType::Image;
        }

        let mut style = self.extract_style(computed_ref);

        let text_content = if node.node_type == DomNodeType::Text {
            let text = node.get_text_content();
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            Some(trimmed.to_string())
        } else {
            None
        };

        if matches!(element_type, ElementType::Text) {
            // For text nodes, prefer inheriting color/family while keeping transparent background.
            style.background_color = None;
        }

        let image_url = if node.node_type == DomNodeType::Element
            && node.tag_name.eq_ignore_ascii_case("img")
        {
            node.get_attribute("src")
        } else {
            None
        };

        Some(LayoutNode {
            node_id,
            bounds: Rect {
                x: layout_box.content_x,
                y: layout_box.content_y,
                width: layout_box.content_width.max(0.0),
                height: layout_box.content_height.max(0.0),
            },
            element_type,
            style,
            text_content,
            image_url,
        })
    }

    fn determine_element_type(
        &self,
        node_type: DomNodeType,
        computed: Option<&ComputedStyles>,
    ) -> Option<ElementType> {
        match node_type {
            DomNodeType::Document | DomNodeType::DocumentType | DomNodeType::Comment => None,
            DomNodeType::Text => Some(ElementType::Text),
            DomNodeType::Element => {
                let display = computed.and_then(|styles| styles.get_computed_value("display").ok());

                match display {
                    Some(ComputedValue::Keyword(keyword)) => {
                        match keyword.to_lowercase().as_str() {
                            "none" => None,
                            "inline" | "inline-block" | "inline-flex" | "inline-grid" => {
                                Some(ElementType::Inline)
                            }
                            "list-item" | "block" | "flex" | "grid" | "table" | "table-row"
                            | "table-cell" => Some(ElementType::Block),
                            _ => Some(ElementType::Block),
                        }
                    }
                    _ => Some(ElementType::Block),
                }
            }
        }
    }

    fn extract_style(&self, computed: Option<&ComputedStyles>) -> Style {
        let mut style = Style::default();

        if let Some(computed) = computed {
            if let Ok(value) = computed.get_computed_value("background-color") {
                if let Some(color) = Self::computed_value_to_color(&value) {
                    if color != "transparent" {
                        style.background_color = Some(color);
                    }
                }
            }

            if let Ok(value) = computed.get_computed_value("color") {
                if let Some(color) = Self::computed_value_to_color(&value) {
                    style.color = Some(color);
                }
            }

            if let Ok(value) = computed.get_computed_value("font-size") {
                if let Some(size) = value.to_f32() {
                    style.font_size = size.max(1.0);
                }
            }

            if let Ok(value) = computed.get_computed_value("font-family") {
                if let Some(family) = Self::computed_value_to_string(&value) {
                    style.font_family = Some(family);
                }
            }
        }

        style
    }

    fn computed_value_to_string(value: &ComputedValue) -> Option<String> {
        match value {
            ComputedValue::String(s) | ComputedValue::Keyword(s) => Some(s.clone()),
            ComputedValue::List(values) => {
                let families: Vec<_> = values
                    .iter()
                    .filter_map(Self::computed_value_to_string)
                    .collect();
                if families.is_empty() {
                    None
                } else {
                    Some(families.join(", "))
                }
            }
            _ => None,
        }
    }

    fn computed_value_to_color(value: &ComputedValue) -> Option<String> {
        match value {
            ComputedValue::Color(color) => Some(Self::color_to_css(color)),
            ComputedValue::Keyword(keyword) if keyword.eq_ignore_ascii_case("transparent") => {
                Some("transparent".to_string())
            }
            ComputedValue::Keyword(keyword) => Some(keyword.clone()),
            ComputedValue::String(value) => Some(value.clone()),
            _ => None,
        }
    }

    fn color_to_css(color: &Color) -> String {
        if (color.a - 1.0).abs() < f32::EPSILON {
            format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
        } else {
            format!("rgba({},{},{},{:.3})", color.r, color.g, color.b, color.a)
        }
    }

    async fn emit_event(&self, event: BrowserEvent) {
        println!("[BrowserEvent] {:?}", event);
        let _ = &self.event_system;
    }

    // Placeholder for memory metric gathering
    async fn get_memory_usage(&self) -> MemoryMetrics {
        // Provide deterministic stub values until wired to a real sampler
        MemoryMetrics {
            heap_size_mb: 0.0,
            used_heap_mb: 0.0,
            gpu_memory_mb: 0.0,
            system_memory_mb: 0.0,
        }
    }
}

impl Drop for BrowserEngine {
    fn drop(&mut self) {
        // Best-effort; prefer explicit async shutdown by the caller.
    }
}

/// Parse the part after "data:" in a data URL. Returns (mime, bytes).
fn parse_data_url(rest: &str) -> std::result::Result<(String, Vec<u8>), String> {
    // RFC 2397: data:[<mediatype>][;base64],<data>
    let idx = rest
        .find(',')
        .ok_or_else(|| "Malformed data URL (missing comma)".to_string())?;
    let (meta, payload) = rest.split_at(idx);
    let payload = &payload[1..]; // skip comma

    // Default media type when omitted
    let mut mime = "text/plain;charset=US-ASCII".to_string();
    let mut base64_flag = false;

    if !meta.is_empty() {
        for part in meta.split(';') {
            if part.eq_ignore_ascii_case("base64") {
                base64_flag = true;
            } else if !part.is_empty() {
                mime = part.to_string();
            }
        }
    }

    let bytes = if base64_flag {
        base64::engine::general_purpose::STANDARD
            .decode(payload)
            .map_err(|_| "Invalid base64 payload".to_string())?
    } else {
        percent_decode_str(payload).collect::<Vec<u8>>()
    };

    Ok((mime, bytes))
}
