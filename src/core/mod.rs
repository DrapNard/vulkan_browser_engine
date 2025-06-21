pub mod css;
pub mod dom;
pub mod events;
pub mod layout;
pub mod network;

use crate::js_engine::JsEngine;
use crate::renderer::VulkanRenderer;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct CoreEngine {
    pub dom: Arc<RwLock<dom::Document>>,
    pub css_engine: css::CssEngine,
    pub layout_engine: layout::LayoutEngine,
    pub js_engine: JsEngine,
    pub renderer: VulkanRenderer,
    pub event_system: events::EventSystem,
    pub network: network::NetworkManager,
}

impl CoreEngine {
    pub async fn new() -> Result<Self, CoreError> {
        let dom = Arc::new(RwLock::new(dom::Document::new()));
        let css_engine = css::CssEngine::new();
        let layout_engine = layout::LayoutEngine::new();
        let js_engine = JsEngine::new().await?;
        let renderer = VulkanRenderer::new().await?;
        let event_system = events::EventSystem::new();
        let network = network::NetworkManager::new();

        Ok(Self {
            dom,
            css_engine,
            layout_engine,
            js_engine,
            renderer,
            event_system,
            network,
        })
    }

    pub async fn load_url(&mut self, url: &str) -> Result<(), CoreError> {
        let html_content = self.network.fetch_html(url).await?;
        let mut dom = self.dom.write().await;
        *dom = self.parse_html(&html_content)?;
        
        self.css_engine.update_styles(&dom);
        self.layout_engine.compute_layout(&dom);
        
        Ok(())
    }

    pub async fn execute_script(&mut self, script: &str) -> Result<serde_json::Value, CoreError> {
        self.js_engine.execute(script).await
            .map_err(CoreError::JsError)
    }

    pub async fn render_frame(&mut self) -> Result<(), CoreError> {
        let dom = self.dom.read().await;
        let layout_tree = self.layout_engine.get_layout_tree();
        self.renderer.render(&*dom, &layout_tree).await
            .map_err(CoreError::RenderError)
    }

    fn parse_html(&self, html: &str) -> Result<dom::Document, CoreError> {
        dom::Document::parse(html)
            .map_err(CoreError::ParseError)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("Network error: {0}")]
    NetworkError(#[from] network::NetworkError),
    #[error("Parse error: {0}")]
    ParseError(dom::ParseError),
    #[error("JavaScript error: {0}")]
    JsError(crate::js_engine::JsError),
    #[error("Render error: {0}")]
    RenderError(crate::renderer::RenderError),
}