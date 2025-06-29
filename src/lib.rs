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
            user_agent: "VulkanBrowser/1.0 (Vulkan; JIT)".to_string(),
            viewport_width: 1920,
            viewport_height: 1080,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PerformanceMetrics {
    pub renderer: RendererMetrics,
    pub javascript: JSMetrics,
    pub layout: LayoutMetrics,
    pub memory_usage: MemoryMetrics,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RendererMetrics {
    pub frame_rate: f64,
    pub render_time_ms: f64,
    pub gpu_utilization: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JSMetrics {
    pub execution_time_ms: f64,
    pub heap_size_mb: f64,
    pub gc_count: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LayoutMetrics {
    pub layout_time_ms: f64,
    pub nodes_count: usize,
    pub reflow_count: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MemoryMetrics {
    pub heap_size_mb: f64,
    pub used_heap_mb: f64,
    pub gpu_memory_mb: f64,
}

#[derive(Debug, Clone)]
pub enum InputEvent {
    MouseMove { x: i32, y: i32 },
    MouseClick { x: i32, y: i32, button: u8 },
    KeyPress { key: String, modifiers: u8 },
    Scroll { delta_x: f64, delta_y: f64 },
    Touch { x: i32, y: i32, pressure: f64 },
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
}

impl BrowserEngine {
    pub async fn new(config: BrowserConfig) -> Result<Self> {
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
        })
    }

    pub async fn load_url(&self, url: &str) -> Result<()> {
        if let Some(_sandbox) = &self.sandbox_manager {
            // Uncomment when check_url_permission is implemented
            // sandbox.check_url_permission(url)?;
        }

        let content = self.network_manager.fetch(url).await?;
        
        {
            let mut document = self.document.write().await;
            document.parse_html(&content)
                .map_err(|e| BrowserError::Document(e.to_string()))?;
        }

        let document_guard = self.document.read().await;
        
        self.style_engine.compute_styles(&*document_guard)
            .map_err(|e| BrowserError::Style(e.to_string()))?;

        {
            let mut layout_engine = self.layout_engine.write().await;
            layout_engine.compute_layout(&*document_guard, &self.style_engine)
                .await
                .map_err(|e| BrowserError::Layout(e.to_string()))?;
        }

        {
            let mut js_runtime = self.js_runtime.write().await;
            js_runtime.inject_document_api(&*document_guard).await?;
            js_runtime.execute_inline_scripts(&*document_guard).await?;
        }

        // Create a layout tree for rendering
        let layout_tree = self.create_layout_tree().await?;
        let mut renderer = self.renderer.write().await;
        renderer.render(&*document_guard, &layout_tree).await?;

        Ok(())
    }

    // Helper method to create layout tree - simplified approach
    async fn create_layout_tree(&self) -> Result<LayoutTree> {
        // Create a new empty layout tree for now
        // You can enhance this to build from actual layout data later
        Ok(LayoutTree::new())
    }

    pub async fn navigate(&self, url: &str) -> Result<()> {
        self.load_url(url).await
    }

    pub async fn execute_javascript(&self, script: &str) -> Result<serde_json::Value> {
        let js_runtime = self.js_runtime.write().await;
        Ok(js_runtime.execute(script).await?)
    }

    pub async fn reload(&self) -> Result<()> {
        let url = {
            let document = self.document.read().await;
            document.get_url().map(|s| s.to_string())
        };
        
        if let Some(url) = url {
            self.load_url(&url).await
        } else {
            Ok(())
        }
    }

    pub async fn resize_viewport(&self, width: u32, height: u32) -> Result<()> {
        {
            let mut layout_engine = self.layout_engine.write().await;
            layout_engine.resize_viewport(width, height)
                .await
                .map_err(|e| BrowserError::Layout(e.to_string()))?;
        }

        let document_guard = self.document.read().await;
        {
            let mut layout_engine = self.layout_engine.write().await;
            layout_engine.compute_layout(&*document_guard, &self.style_engine)
                .await
                .map_err(|e| BrowserError::Layout(e.to_string()))?;
        }
        
        let layout_tree = self.create_layout_tree().await?;
        let mut renderer = self.renderer.write().await;
        renderer.render(&*document_guard, &layout_tree).await?;
        
        Ok(())
    }

    pub async fn get_performance_metrics(&self) -> PerformanceMetrics {
        // Create placeholder renderer metrics since the actual type doesn't match
        let renderer_metrics = RendererMetrics {
            frame_rate: 60.0,
            render_time_ms: 16.7,
            gpu_utilization: 0.0,
        };
        
        let js_perf_metrics = self.js_runtime.read().await.get_metrics().await;
        let js_metrics = JSMetrics {
            execution_time_ms: js_perf_metrics.execution_time_us as f64 / 1000.0,
            heap_size_mb: js_perf_metrics.heap_size_bytes as f64 / (1024.0 * 1024.0),
            gc_count: 0,
        };
        
        let layout_perf_metrics = self.layout_engine.read().await.get_metrics().await;
        let layout_metrics = LayoutMetrics {
            layout_time_ms: layout_perf_metrics.average_layout_time_us as f64 / 1000.0,
            nodes_count: 0,
            reflow_count: layout_perf_metrics.total_layouts,
        };
        
        let memory_metrics = self.get_memory_usage().await;

        PerformanceMetrics {
            renderer: renderer_metrics,
            javascript: js_metrics,
            layout: layout_metrics,
            memory_usage: memory_metrics,
        }
    }

    async fn get_memory_usage(&self) -> MemoryMetrics {
        let gpu_memory = 0.0;
        
        MemoryMetrics {
            heap_size_mb: 0.0,
            used_heap_mb: 0.0,
            gpu_memory_mb: gpu_memory,
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

    pub async fn handle_input_event(&self, _event: InputEvent) -> Result<()> {
        // For now, we'll skip event conversion until the Event enum is properly defined
        // You can add logging here if needed: eprintln!("Received input event: {:?}", _event);
        
        // TODO: Implement proper event conversion based on your actual Event enum
        // let converted_event = self.convert_input_event(_event)?;
        // self.event_system.dispatch_event(converted_event).await;
        
        Ok(())
    }

    // Helper method to convert InputEvent to Event (placeholder for future implementation)
    #[allow(dead_code)]
    fn convert_input_event(&self, _input_event: InputEvent) -> Result<crate::core::events::Event> {
        // TODO: Implement this based on your actual Event enum structure
        Err(BrowserError::Platform("Event conversion not yet implemented".to_string()))
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
            _ => return Err(BrowserError::Platform(format!("Unknown API: {}", api_name))),
        }

        Ok(())
    }

    pub async fn shutdown(&self) -> Result<()> {
        if let Some(_sandbox) = &self.sandbox_manager {
            // Uncomment when shutdown method is implemented
            // sandbox.shutdown().await?;
        }

        if let Some(pwa) = &self.pwa_manager {
            pwa.shutdown().await?;
        }

        self.js_runtime.write().await.shutdown().await?;
        // Uncomment when renderer shutdown is implemented
        // self.renderer.write().await.shutdown().await?;
        self.network_manager.shutdown().await?;

        Ok(())
    }
}