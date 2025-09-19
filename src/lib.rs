use std::sync::Arc;
use tokio::sync::RwLock;
use thiserror::Error;
use serde::{Serialize, Deserialize};

pub mod core;
pub mod js_engine;
pub mod renderer;
pub mod sandbox;
pub mod pwa;

use crate::core::{
    dom::Document,
    events::EventSystem,
    network::{NetworkManager, NetworkError},
    css::StyleEngine,
    layout::LayoutEngine,
};
use crate::js_engine::{JSRuntime, JSError};
use crate::renderer::{VulkanRenderer, RenderError, LayoutTree};
use crate::sandbox::{SandboxManager, SandboxError};
use crate::pwa::PwaError;
use crate::pwa::PwaRuntime as PwaManager;

#[derive(Error, Debug)]
pub enum BrowserError {
    #[error("Renderer initialization failed: {0}")]
    RendererInit(String),
    #[error("JavaScript engine error: {0}")]
    JSEngine(#[from] JSError),
    #[error("Network error: {0}")]
    Network(#[from] NetworkError),
    #[error("Sandbox violation: {0}")]
    Sandbox(#[from] SandboxError),
    #[error("PWA error: {0}")]
    PWA(#[from] PwaError),
    #[error("Render error: {0}")]
    Render(#[from] RenderError),
    #[error("Document parsing error: {0}")]
    Document(String),
    #[error("Layout error: {0}")]
    Layout(String),
    #[error("Style computation error: {0}")]
    Style(String),
    #[error("Platform error: {0}")]
    Platform(String),
}

pub type Result<T> = std::result::Result<T, BrowserError>;

#[derive(Debug, Clone)]
pub struct BrowserConfig {
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
            allow_data_urls: false,
max_data_url_bytes: 256 * 1024, // 256 KiB cap
allowed_data_mime_prefixes: vec![
    "text/html".to_string(),
    "text/plain".to_string(),
    "image/".to_string(),
    "font/".to_string(),
    "application/javascript".to_string(),
    "text/css".to_string(),
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
    MouseMove { x: i32, y: i32 },
    MouseClick { x: i32, y: i32, button: u8 },
    MouseWheel { x: i32, y: i32, delta_x: f64, delta_y: f64 },
    KeyPress { key: String, modifiers: u8 },
    KeyRelease { key: String, modifiers: u8 },
    Scroll { delta_x: f64, delta_y: f64 },
    Touch { x: i32, y: i32, pressure: f64, id: u32 },
    Resize { width: u32, height: u32 },
}

#[derive(Debug, Clone)]
pub enum BrowserEvent {
    PageLoaded { url: String, load_time_ms: u64 },
    NavigationStarted { url: String },
    JavaScriptError { message: String, line: u32, column: u32 },
    NetworkError { url: String, error: String },
    SecurityViolation { description: String },
    PerformanceWarning { metric: String, value: f64, threshold: f64 },
}

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
}

impl BrowserEngine {
    pub async fn new(config: BrowserConfig) -> Result<Self> {
        // Initialize V8 first - this should be done before creating JSRuntime
        if !crate::js_engine::v8_binding::V8Runtime::is_v8_initialized() {
            // V8 will be initialized automatically when first V8Runtime is created
        }

        let renderer = Arc::new(RwLock::new(
            VulkanRenderer::new()
                .await
                .map_err(|e| BrowserError::RendererInit(e.to_string()))?
        ));

        let js_runtime = Arc::new(RwLock::new(
            JSRuntime::new(&config).await?
        ));

        let document = Arc::new(RwLock::new(Document::new()));
        let style_engine = Arc::new(StyleEngine::new());
        let layout_engine = Arc::new(RwLock::new(
            LayoutEngine::new(config.viewport_width, config.viewport_height)
        ));
        let event_system = Arc::new(EventSystem::new());
        let network_manager = Arc::new(NetworkManager::new(&config).await?);

        let sandbox_manager = if config.enable_sandbox {
            Some(Arc::new(SandboxManager::new().await?))
        } else {
            None
        };

        let pwa_manager = if config.enable_pwa {
            Some(Arc::new(PwaManager::new().await?))
        } else {
            None
        };

        let is_shutdown = Arc::new(RwLock::new(false));

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
            is_shutdown,
        })
    }

    pub async fn load_url(&self, url: &str) -> Result<()> {
        if *self.is_shutdown.read().await {
            return Err(BrowserError::Platform("Browser engine has been shut down".to_string()));
        }

        // Security check
        if let Some(_sandbox) = &self.sandbox_manager {
            // Uncomment when check_url_permission is implemented
            // sandbox.check_url_permission(url)?;
        }

        // Emit navigation started event
        self.emit_event(BrowserEvent::NavigationStarted { 
            url: url.to_string() 
        }).await;

        let start_time = std::time::Instant::now();

        // Fetch content
        let content = self.network_manager.fetch(url).await?;
        
        // Parse HTML and update document
        {
            let document = self.document.write().await;
            document.parse_html(&content)
                .map_err(|e| BrowserError::Document(e.to_string()))?;
            document.set_url(url.to_string()); // Fixed: convert &str to String
        }

        let document_guard = self.document.read().await;
        
        // Compute styles
        self.style_engine.compute_styles(&document_guard)
            .map_err(|e| BrowserError::Style(e.to_string()))?;

        // Compute layout
        {
            let layout_engine = self.layout_engine.write().await;
            layout_engine.compute_layout(&document_guard, &self.style_engine)
                .await
                .map_err(|e| BrowserError::Layout(e.to_string()))?;
        }

        // Execute JavaScript
        {
            let js_runtime = self.js_runtime.write().await;
            js_runtime.inject_document_api(&document_guard).await?;
            
            // Execute inline scripts with error handling
            if let Err(e) = js_runtime.execute_inline_scripts(&document_guard).await {
                self.emit_event(BrowserEvent::JavaScriptError {
                    message: e.to_string(),
                    line: 0,
                    column: 0,
                }).await;
                // Continue execution despite JS errors
            }
        }

        // Render the page
        let layout_tree = self.create_layout_tree().await?;
        {
            let mut renderer = self.renderer.write().await;
            renderer.render(&document_guard, &layout_tree).await?;
        }

        let load_time = start_time.elapsed().as_millis() as u64;
        
        // Emit page loaded event
        self.emit_event(BrowserEvent::PageLoaded {
            url: url.to_string(),
            load_time_ms: load_time,
        }).await;

        Ok(())
    }

    async fn create_layout_tree(&self) -> Result<LayoutTree> {
        // Create a layout tree from the current document and layout state
        // This is a simplified implementation
        Ok(LayoutTree::new())
    }

    async fn emit_event(&self, _event: BrowserEvent) {
        // Event emission implementation
        // You can log events, send to event handlers, etc.
        // For now, this is a placeholder
    }

    pub async fn navigate(&self, url: &str) -> Result<()> {
        // Simply delegate to load_url without recursion
        self.load_url(url).await
    }

    pub async fn navigate_back(&self) -> Result<()> {
        // Simplified back navigation - you'd implement proper history management
        Err(BrowserError::Platform("Back navigation not yet implemented".to_string()))
    }

    pub async fn navigate_forward(&self) -> Result<()> {
        // Simplified forward navigation - you'd implement proper history management
        Err(BrowserError::Platform("Forward navigation not yet implemented".to_string()))
    }

    pub async fn execute_javascript(&self, script: &str) -> Result<serde_json::Value> {
        if *self.is_shutdown.read().await {
            return Err(BrowserError::Platform("Browser engine has been shut down".to_string()));
        }

        let js_runtime = self.js_runtime.write().await;
        match js_runtime.execute(script).await {
            Ok(result) => Ok(result),
            Err(e) => {
                self.emit_event(BrowserEvent::JavaScriptError {
                    message: e.to_string(),
                    line: 0,
                    column: 0,
                }).await;
                Err(e.into())
            }
        }
    }

    pub async fn reload(&self) -> Result<()> {
        let url = {
            let document = self.document.read().await;
            document.get_url().map(|s| s.to_string())
        };
        
        if let Some(url) = url {
            // Simply delegate to load_url without recursion
            self.load_url(&url).await
        } else {
            Err(BrowserError::Platform("No URL to reload".to_string()))
        }
    }

    pub async fn resize_viewport(&self, width: u32, height: u32) -> Result<()> {
        if *self.is_shutdown.read().await {
            return Err(BrowserError::Platform("Browser engine has been shut down".to_string()));
        }

        // Update layout engine viewport
        {
            let layout_engine = self.layout_engine.write().await;
            layout_engine.resize_viewport(width, height)
                .await
                .map_err(|e| BrowserError::Layout(e.to_string()))?;
        }

        // Recompute layout with new viewport
        let document_guard = self.document.read().await;
        {
            let layout_engine = self.layout_engine.write().await;
            layout_engine.compute_layout(&document_guard, &self.style_engine)
                .await
                .map_err(|e| BrowserError::Layout(e.to_string()))?;
        }
        
        // Re-render with new layout
        let layout_tree = self.create_layout_tree().await?;
        {
            let mut renderer = self.renderer.write().await;
            renderer.render(&document_guard, &layout_tree).await?;
        }
        
        // Note: We don't call handle_input_event here to avoid recursion
        // The resize event is already handled by this method
        
        Ok(())
    }

    pub async fn get_performance_metrics(&self) -> PerformanceMetrics {
        // Collect renderer metrics
        let renderer_metrics = RendererMetrics {
            frame_rate: 60.0, // Default placeholder
            render_time_ms: 16.7,
            gpu_utilization: 0.0,
            draw_calls: 0,
            triangles_rendered: 0,
        };
        
        // Collect JS metrics
        let js_perf_metrics = self.js_runtime.read().await.get_metrics().await;
        let js_metrics = JSMetrics {
            execution_time_ms: js_perf_metrics.execution_time_us as f64 / 1000.0,
            heap_size_mb: js_perf_metrics.heap_size_bytes as f64 / (1024.0 * 1024.0),
            gc_count: 0,
            compile_time_ms: 0.0,
            active_isolates: 1,
        };
        
        // Collect layout metrics
        let layout_perf_metrics = self.layout_engine.read().await.get_metrics().await;
        let layout_metrics = LayoutMetrics {
            layout_time_ms: layout_perf_metrics.average_layout_time_us as f64 / 1000.0,
            nodes_count: 0,
            reflow_count: layout_perf_metrics.total_layouts,
            style_recalc_time_ms: 0.0,
        };
        
        // Collect memory metrics
        let memory_metrics = self.get_memory_usage().await;

        // Collect network metrics
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

    async fn get_memory_usage(&self) -> MemoryMetrics {
        let gpu_memory = 0.0; // Placeholder
        
        MemoryMetrics {
            heap_size_mb: 0.0,
            used_heap_mb: 0.0,
            gpu_memory_mb: gpu_memory,
            system_memory_mb: 0.0,
        }
    }

    pub async fn install_pwa(&self, manifest_url: &str) -> Result<()> {
        if let Some(pwa_manager) = &self.pwa_manager {
            let manifest_content = self.network_manager.fetch(manifest_url).await?;
            let manifest: crate::pwa::manifest::Manifest = serde_json::from_str(&manifest_content)
                .map_err(|e| BrowserError::Platform(format!("Failed to parse manifest: {}", e)))?;
            
            let _app_id = pwa_manager.install_app(&manifest).await?;
            Ok(())
        } else {
            Err(BrowserError::Platform("PWA functionality not enabled".to_string()))
        }
    }

    pub async fn register_service_worker(&self, script_url: &str) -> Result<()> {
        if let Some(pwa_manager) = &self.pwa_manager {
            let _worker_id = pwa_manager.register_service_worker(script_url, None).await?;
            Ok(())
        } else {
            Err(BrowserError::Platform("PWA functionality not enabled".to_string()))
        }
    }

    pub async fn handle_input_event(&self, event: InputEvent) -> Result<()> {
        if *self.is_shutdown.read().await {
            return Err(BrowserError::Platform("Browser engine has been shut down".to_string()));
        }

        // Process different types of input events
        match event {
            InputEvent::MouseMove { x: _, y: _ } => {
                // Handle mouse movement
                // Update cursor position, handle hover effects, etc.
            },
            InputEvent::MouseClick { x, y, button: _ } => {
                // Handle mouse clicks
                // Determine what element was clicked, trigger onclick events, etc.
                let _click_target = self.find_element_at_position(x, y).await;
            },
            InputEvent::KeyPress { key: _, modifiers: _ } => {
                // Handle keyboard input
                // Update focused element, handle shortcuts, etc.
            },
            InputEvent::Scroll { delta_x: _, delta_y: _ } => {
                // Handle scrolling
                // Update viewport position, trigger scroll events, etc.
            },
            InputEvent::Touch { x: _, y: _, pressure: _, id: _ } => {
                // Handle touch input
                // Similar to mouse but with touch-specific handling
            },
            InputEvent::Resize { width, height } => {
                // Handle resize (already implemented above)
                return self.resize_viewport(width, height).await;
            },
            _ => {
                // Handle other input types
            }
        }
        
        Ok(())
    }

    async fn find_element_at_position(&self, _x: i32, _y: i32) -> Option<String> {
        // Placeholder for hit testing implementation
        // You would traverse the layout tree to find what element is at the given position
        None
    }

    pub async fn enable_chrome_api(&self, api_name: &str) -> Result<()> {
        if !self.config.enable_chrome_apis {
            return Err(BrowserError::Platform("Chrome APIs not enabled".to_string()));
        }

        let js_runtime = self.js_runtime.write().await;
        
        match api_name {
            "serial" => js_runtime.inject_serial_api().await?,
            "usb" => js_runtime.inject_usb_api().await?,
            "bluetooth" => js_runtime.inject_bluetooth_api().await?,
            "gamepad" => js_runtime.inject_gamepad_api().await?,
            "webrtc" => js_runtime.inject_webrtc_api().await?,
            "websocket" => js_runtime.inject_websocket_api().await?,
            // Commented out APIs that don't exist yet
            // "geolocation" => js_runtime.inject_geolocation_api().await?,
            // "notifications" => js_runtime.inject_notifications_api().await?,
            // "storage" => js_runtime.inject_storage_api().await?,
            _ => return Err(BrowserError::Platform(format!("Unknown or unimplemented API: {}", api_name))),
        }

        Ok(())
    }

    pub async fn set_user_agent(&self, _user_agent: &str) -> Result<()> {
        // Update the user agent for future requests
        // Note: NetworkManager doesn't have set_user_agent method yet
        // self.network_manager.set_user_agent(user_agent).await?;
        
        // For now, return an error indicating this is not implemented
        Err(BrowserError::Platform("set_user_agent not yet implemented in NetworkManager".to_string()))
    }

    pub async fn clear_cache(&self) -> Result<()> {
        // Clear various caches
        // Note: NetworkManager clear_cache method doesn't exist yet
        // self.network_manager.clear_cache().await?;
        
        // Clear JS runtime caches (method doesn't exist yet)
        // {
        //     let js_runtime = self.js_runtime.write().await;
        //     js_runtime.clear_cache().await?;
        // }
        
        // For now, return an error indicating this is not implemented
        Err(BrowserError::Platform("clear_cache not yet implemented".to_string()))
    }

    pub async fn get_current_url(&self) -> Option<String> {
        let document = self.document.read().await;
        document.get_url().map(|s| s.to_string())
    }

    pub async fn get_page_title(&self) -> Option<String> {
        let document = self.document.read().await;
        // If get_title() returns String instead of Option<String>, wrap it in Some
        Some(document.get_title())
    }

    pub async fn is_loading(&self) -> bool {
        // Implement loading state tracking
        false // Placeholder
    }

    pub async fn shutdown(&self) -> Result<()> {
        // Set shutdown flag
        {
            let mut shutdown_guard = self.is_shutdown.write().await;
            if *shutdown_guard {
                return Ok(()); // Already shut down
            }
            *shutdown_guard = true;
        }

        // Shutdown components in reverse order of initialization
        if let Some(pwa) = &self.pwa_manager {
            pwa.shutdown().await?;
        }

        if let Some(_sandbox) = &self.sandbox_manager {
            // Uncomment when shutdown method is implemented
            // sandbox.shutdown().await?;
        }

        // Shutdown JS runtime
        self.js_runtime.write().await.shutdown().await?;

        // Shutdown network manager
        self.network_manager.shutdown().await?;

        // Shutdown renderer
        // Uncomment when renderer shutdown is implemented
        // self.renderer.write().await.shutdown().await?;

        // First shutdown JS runtime to drop isolates and contexts
        {
            let rt = self.js_runtime.read().await;
            rt.shutdown().await.map_err(|e| BrowserError::Platform(e.to_string()))?;
        }
        // Dispose V8 global state - this should be done last and only once per process
        crate::js_engine::v8_binding::V8Runtime::dispose_v8();

        Ok(())
    }
}

impl Drop for BrowserEngine {
    fn drop(&mut self) {
        // Note: We can't call async shutdown in Drop, so we rely on explicit shutdown
        // or process termination to clean up V8
    }
}