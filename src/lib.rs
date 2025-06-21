use std::sync::Arc;
use tokio::sync::RwLock;
use parking_lot::Mutex;
use thiserror::Error;

pub mod core;
pub mod js_engine;
pub mod platform;
pub mod pwa;
pub mod renderer;
pub mod sandbox;

use crate::core::{
    dom::Document,
    events::EventSystem,
    network::NetworkManager,
    css::StyleEngine,
    layout::LayoutEngine,
};
use crate::js_engine::JSRuntime;
use crate::renderer::VulkanRenderer;
use crate::sandbox::SandboxManager;
use crate::pwa::PWAManager;

#[derive(Error, Debug)]
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

pub struct BrowserEngine {
    config: BrowserConfig,
    renderer: Arc<VulkanRenderer>,
    js_runtime: Arc<RwLock<JSRuntime>>,
    document: Arc<RwLock<Document>>,
    style_engine: Arc<StyleEngine>,
    layout_engine: Arc<LayoutEngine>,
    event_system: Arc<EventSystem>,
    network_manager: Arc<NetworkManager>,
    sandbox_manager: Option<Arc<SandboxManager>>,
    pwa_manager: Option<Arc<PWAManager>>,
}

impl BrowserEngine {
    pub async fn new(config: BrowserConfig) -> Result<Self> {
        let renderer = Arc::new(
            VulkanRenderer::new(&config)
                .await
                .map_err(|e| BrowserError::RendererInit(e.to_string()))?
        );

        let js_runtime = Arc::new(RwLock::new(
            JSRuntime::new(&config)
                .await
                .map_err(|e| BrowserError::JSEngine(e.to_string()))?
        ));

        let document = Arc::new(RwLock::new(Document::new()));
        let style_engine = Arc::new(StyleEngine::new());
        let layout_engine = Arc::new(LayoutEngine::new(config.viewport_width, config.viewport_height));
        let event_system = Arc::new(EventSystem::new());
        let network_manager = Arc::new(NetworkManager::new(&config).await?);

        let sandbox_manager = if config.enable_sandbox {
            Some(Arc::new(SandboxManager::new(&config).await?))
        } else {
            None
        };

        let pwa_manager = if config.enable_pwa {
            Some(Arc::new(PWAManager::new(&config).await?))
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
        if let Some(sandbox) = &self.sandbox_manager {
            sandbox.check_url_permission(url)?;
        }

        let content = self.network_manager.fetch(url).await?;
        let mut document = self.document.write().await;
        document.parse_html(&content)?;

        self.style_engine.compute_styles(&document).await?;
        self.layout_engine.compute_layout(&document, &self.style_engine).await?;

        let mut js_runtime = self.js_runtime.write().await;
        js_runtime.inject_document_api(&document).await?;
        js_runtime.execute_inline_scripts(&document).await?;

        self.renderer.render(&document, &self.layout_engine).await?;

        Ok(())
    }

    pub async fn navigate(&self, url: &str) -> Result<()> {
        self.load_url(url).await
    }

    pub async fn execute_javascript(&self, script: &str) -> Result<serde_json::Value> {
        let mut js_runtime = self.js_runtime.write().await;
        js_runtime.execute(script).await
            .map_err(|e| BrowserError::JSEngine(e.to_string()))
    }

    pub async fn reload(&self) -> Result<()> {
        let document = self.document.read().await;
        if let Some(url) = document.get_url() {
            drop(document);
            self.load_url(&url).await
        } else {
            Ok(())
        }
    }

    pub async fn resize_viewport(&self, width: u32, height: u32) -> Result<()> {
        self.layout_engine.resize_viewport(width, height).await?;
        let document = self.document.read().await;
        self.layout_engine.compute_layout(&document, &self.style_engine).await?;
        self.renderer.render(&document, &self.layout_engine).await?;
        Ok(())
    }

    pub async fn get_performance_metrics(&self) -> serde_json::Value {
        let renderer_metrics = self.renderer.get_metrics().await;
        let js_metrics = self.js_runtime.read().await.get_metrics().await;
        let layout_metrics = self.layout_engine.get_metrics().await;

        serde_json::json!({
            "renderer": renderer_metrics,
            "javascript": js_metrics,
            "layout": layout_metrics,
            "memory_usage": self.get_memory_usage().await,
        })
    }

    async fn get_memory_usage(&self) -> serde_json::Value {
        serde_json::json!({
            "heap_size": 0,
            "used_heap": 0,
            "gpu_memory": self.renderer.get_memory_usage().await,
        })
    }

    pub async fn install_pwa(&self, manifest_url: &str) -> Result<()> {
        if let Some(pwa_manager) = &self.pwa_manager {
            pwa_manager.install_pwa(manifest_url).await
                .map_err(|e| BrowserError::PWA(e.to_string()))
        } else {
            Err(BrowserError::PWA("PWA support not enabled".to_string()))
        }
    }

    pub async fn register_service_worker(&self, script_url: &str) -> Result<()> {
        if let Some(pwa_manager) = &self.pwa_manager {
            pwa_manager.register_service_worker(script_url).await
                .map_err(|e| BrowserError::PWA(e.to_string()))
        } else {
            Err(BrowserError::PWA("PWA support not enabled".to_string()))
        }
    }

    pub async fn handle_input_event(&self, event: platform::InputEvent) -> Result<()> {
        self.event_system.dispatch_event(event).await;
        Ok(())
    }

    pub async fn enable_chrome_api(&self, api_name: &str) -> Result<()> {
        if !self.config.enable_chrome_apis {
            return Err(BrowserError::Platform("Chrome APIs not enabled".to_string()));
        }

        let mut js_runtime = self.js_runtime.write().await;
        
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
        if let Some(sandbox) = &self.sandbox_manager {
            sandbox.shutdown().await?;
        }

        if let Some(pwa) = &self.pwa_manager {
            pwa.shutdown().await?;
        }

        self.js_runtime.write().await.shutdown().await?;
        self.renderer.shutdown().await?;
        self.network_manager.shutdown().await?;

        Ok(())
    }
}