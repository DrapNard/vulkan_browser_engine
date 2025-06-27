pub mod css;
pub mod dom;
pub mod events;
pub mod layout;
pub mod network;

use crate::js_engine::{JSRuntime, JSError};
use crate::renderer::{VulkanRenderer, RenderError, LayoutTree};
use css::computed::StyleEngine;
use dom::Document;
use events::EventSystem;
use layout::LayoutEngine;
use network::{NetworkManager};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::BrowserConfig;

// Converts a core::layout::LayoutBox to a renderer::LayoutTree
fn from_layout_box(layout_box: &layout::LayoutBox) -> LayoutTree {
    // TODO: Implement actual conversion logic
    LayoutTree::from_layout_box(layout_box)
}

pub struct CoreEngine {
    pub dom: Arc<RwLock<Document>>,
    pub style_engine: StyleEngine,
    pub layout_engine: LayoutEngine,
    pub js_engine: JSRuntime,
    pub renderer: VulkanRenderer,
    pub event_system: EventSystem,
    pub network: NetworkManager,
}

#[derive(thiserror::Error, Debug)]
pub enum CoreError {
    #[error("Network error: {0}")]
    NetworkError(#[from] network::NetworkError),
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("JavaScript error: {0}")]
    JsError(#[from] JSError),
    #[error("Render error: {0}")]
    RenderError(#[from] RenderError),
}

impl CoreEngine {
    pub async fn new(config: &BrowserConfig, width: u32, height: u32) -> Result<Self, CoreError> {
        let dom = Arc::new(RwLock::new(Document::new()));
        let style_engine = StyleEngine::new();
        let layout_engine = LayoutEngine::new(width, height);
        let js_engine = JSRuntime::new(config).await?;
        let renderer = VulkanRenderer::new().await?;
        let event_system = EventSystem::new();
        let network = NetworkManager::new(config).await?;

        Ok(Self {
            dom,
            style_engine,
            layout_engine,
            js_engine,
            renderer,
            event_system,
            network,
        })
    }

    pub async fn load_url(&mut self, url: &str) -> Result<(), CoreError> {
        let html = self.network.fetch(url).await?;
        let mut doc_guard = self.dom.write().await;
        *doc_guard = Document::parse(&html)
            .map_err(|e| CoreError::ParseError(e.to_string()))?;
        self.style_engine.compute_styles(&*doc_guard);
        self.layout_engine.compute_layout(&*doc_guard, &self.style_engine);
        Ok(())
    }

    pub async fn execute_script(&mut self, script: &str) -> Result<Value, CoreError> {
        Ok(self.js_engine.execute(script).await?)
    }

    pub async fn render_frame(&mut self) -> Result<(), CoreError> {
        let doc_guard = self.dom.read().await;
        let root_node_id = doc_guard.get_root_node().ok_or_else(|| CoreError::ParseError("No root node found".to_string()))?;
        let layout_root = self.layout_engine.get_layout_box(root_node_id);
        let layout_box = layout_root.as_ref().ok_or_else(|| CoreError::ParseError("No layout tree found".to_string()))?;
        let layout_tree = from_layout_box(layout_box); // Convert to renderer::LayoutTree
        self.renderer.render(&*doc_guard, &layout_tree).await?;
        Ok(())
    }
}
