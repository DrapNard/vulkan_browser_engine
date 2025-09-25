pub mod document;
pub mod element;
pub mod node;

pub use document::{
    Document, DocumentError, DocumentMetadata, DocumentReadyState, InlineScript, MutationRecord,
    MutationType, NodeId,
};
pub use element::{
    AnimationId, AnimationOptions, DOMRect, Element, ElementError, ShadowRootInit, ShadowRootMode,
};
pub use node::{
    AttributeMap, ClearType, ComputedStyle, DisplayType, FloatType, LayoutData, Node, NodeType,
    OverflowType, PositionType,
};

use crate::core::dom::document::NodeType as DocumentNodeType;
use parking_lot::RwLock;
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DOMError {
    #[error("Document error: {0}")]
    Document(#[from] DocumentError),
    #[error("Element error: {0}")]
    Element(#[from] ElementError),
    #[error("Node operation failed: {0}")]
    NodeOperation(String),
    #[error("Tree manipulation failed: {0}")]
    TreeManipulation(String),
    #[error("Event handling failed: {0}")]
    EventHandling(String),
}

pub type Result<T> = std::result::Result<T, DOMError>;

#[derive(Debug, Clone)]
pub struct DOMImplementation {
    feature_support: Arc<RwLock<FeatureSupport>>,
}

#[derive(Debug, Clone)]
pub struct FeatureSupport {
    pub html: bool,
    pub xml: bool,
    pub core: bool,
    pub svg: bool,
    pub mathml: bool,
    pub traversal: bool,
    pub range: bool,
    pub events: bool,
    pub mutation_events: bool,
    pub css: bool,
    pub css2: bool,
    pub css3: bool,
    pub xpath: bool,
}

impl Default for FeatureSupport {
    fn default() -> Self {
        Self {
            html: true,
            xml: true,
            core: true,
            svg: true,
            mathml: false,
            traversal: true,
            range: true,
            events: true,
            mutation_events: true,
            css: true,
            css2: true,
            css3: true,
            xpath: false,
        }
    }
}

impl Default for DOMImplementation {
    fn default() -> Self {
        Self::new()
    }
}

impl DOMImplementation {
    pub fn new() -> Self {
        Self {
            feature_support: Arc::new(RwLock::new(FeatureSupport::default())),
        }
    }

    pub fn has_feature(&self, feature: &str, _version: &str) -> bool {
        let support = self.feature_support.read();
        match feature.to_lowercase().as_str() {
            "html" => support.html,
            "xml" => support.xml,
            "core" => support.core,
            "svg" => support.svg,
            "mathml" => support.mathml,
            "traversal" => support.traversal,
            "range" => support.range,
            "events" => support.events,
            "mutationevents" => support.mutation_events,
            "css" => support.css,
            "css2" => support.css2,
            "css3" => support.css3,
            "xpath" => support.xpath,
            _ => false,
        }
    }

    pub fn create_document_type(
        &self,
        qualified_name: &str,
        _public_id: &str,
        _system_id: &str,
    ) -> Result<Node> {
        Ok(Node::new_doctype(qualified_name.to_string(), NodeId::new()))
    }

    pub fn create_document(
        &self,
        _namespace_uri: Option<&str>,
        qualified_name: Option<&str>,
    ) -> Result<Document> {
        let document = Document::new();
        if let Some(name) = qualified_name {
            let root_el = document.create_node(DocumentNodeType::Element, name.to_string())?;
            if let Some(root_id) = document.get_root_node() {
                document.append_child(root_id, root_el)?;
            }
        }
        Ok(document)
    }

    pub fn create_html_document(&self, title: Option<&str>) -> Result<Document> {
        let document = Document::new();
        let html_id = document.create_node(DocumentNodeType::Element, "html".into())?;
        let head_id = document.create_node(DocumentNodeType::Element, "head".into())?;
        let body_id = document.create_node(DocumentNodeType::Element, "body".into())?;
        if let Some(text) = title {
            let title_id = document.create_node(DocumentNodeType::Element, "title".into())?;
            let text_id = document.create_node(DocumentNodeType::Text, text.to_string())?;
            document.append_child(title_id, text_id)?;
            document.append_child(head_id, title_id)?;
            document.set_title(text.to_string());
        }
        document.append_child(html_id, head_id)?;
        document.append_child(html_id, body_id)?;
        if let Some(root_id) = document.get_root_node() {
            document.append_child(root_id, html_id)?;
        }
        Ok(document)
    }
}

pub struct TreeWalker {
    root: NodeId,
    what_to_show: u32,
    filter: Option<Arc<dyn Fn(NodeId) -> bool + Send + Sync>>,
    current_node: NodeId,
}

impl TreeWalker {
    pub fn new(root: NodeId, what_to_show: u32) -> Self {
        Self {
            root,
            what_to_show,
            filter: None,
            current_node: root,
        }
    }

    pub fn with_filter<F>(mut self, filter: F) -> Self
    where
        F: Fn(NodeId) -> bool + Send + Sync + 'static,
    {
        self.filter = Some(Arc::new(filter));
        self
    }

    pub fn get_root(&self) -> NodeId {
        self.root
    }

    pub fn get_current_node(&self) -> NodeId {
        self.current_node
    }

    pub fn set_current_node(&mut self, node: NodeId) {
        self.current_node = node;
    }

    pub fn parent_node(&mut self, document: &Document) -> Option<NodeId> {
        let p = document.get_parent(self.current_node)?;
        if self.accepts_node(p) {
            self.current_node = p;
            Some(p)
        } else {
            None
        }
    }

    pub fn first_child(&mut self, document: &Document) -> Option<NodeId> {
        for &c in &document.get_children(self.current_node) {
            if self.accepts_node(c) {
                self.current_node = c;
                return Some(c);
            }
        }
        None
    }

    pub fn last_child(&mut self, document: &Document) -> Option<NodeId> {
        for &c in document.get_children(self.current_node).iter().rev() {
            if self.accepts_node(c) {
                self.current_node = c;
                return Some(c);
            }
        }
        None
    }

    pub fn next_sibling(&mut self, document: &Document) -> Option<NodeId> {
        let parent = document.get_parent(self.current_node)?;
        let siblings = document.get_children(parent);
        let idx = siblings.iter().position(|&id| id == self.current_node)?;
        for &sib in &siblings[idx + 1..] {
            if self.accepts_node(sib) {
                self.current_node = sib;
                return Some(sib);
            }
        }
        None
    }

    pub fn previous_sibling(&mut self, document: &Document) -> Option<NodeId> {
        let parent = document.get_parent(self.current_node)?;
        let siblings = document.get_children(parent);
        let idx = siblings.iter().position(|&id| id == self.current_node)?;
        for &sib in siblings[..idx].iter().rev() {
            if self.accepts_node(sib) {
                self.current_node = sib;
                return Some(sib);
            }
        }
        None
    }

    pub fn next_node(&mut self, document: &Document) -> Option<NodeId> {
        if let Some(c) = self.first_child(document) {
            return Some(c);
        }
        if let Some(s) = self.next_sibling(document) {
            return Some(s);
        }
        let mut curr = self.current_node;
        while let Some(p) = document.get_parent(curr) {
            if p == self.root {
                break;
            }
            self.current_node = p;
            if let Some(sib) = self.next_sibling(document) {
                return Some(sib);
            }
            curr = p;
        }
        None
    }

    pub fn previous_node(&mut self, document: &Document) -> Option<NodeId> {
        if let Some(sib) = self.previous_sibling(document) {
            self.current_node = sib;
            while let Some(child) = self.last_child(document) {
                let _ = child;
            }
            return Some(self.current_node);
        }
        if let Some(p) = self.parent_node(document) {
            if p != self.root {
                return Some(p);
            }
        }
        None
    }

    fn accepts_node(&self, node: NodeId) -> bool {
        if let Some(ref f) = self.filter {
            f(node)
        } else {
            true
        }
    }
}

impl std::fmt::Debug for TreeWalker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeWalker")
            .field("root", &self.root)
            .field("what_to_show", &self.what_to_show)
            .field("current_node", &self.current_node)
            .finish()
    }
}

pub struct NodeIterator {
    root: NodeId,
    what_to_show: u32,
    filter: Option<Arc<dyn Fn(NodeId) -> bool + Send + Sync>>,
    reference_node: NodeId,
    pointer_before_reference_node: bool,
}

impl NodeIterator {
    pub fn new(root: NodeId, what_to_show: u32) -> Self {
        Self {
            root,
            what_to_show,
            filter: None,
            reference_node: root,
            pointer_before_reference_node: true,
        }
    }

    pub fn with_filter<F>(mut self, filter: F) -> Self
    where
        F: Fn(NodeId) -> bool + Send + Sync + 'static,
    {
        self.filter = Some(Arc::new(filter));
        self
    }

    pub fn next_node(&mut self, _document: &Document) -> Option<NodeId> {
        None
    }

    pub fn previous_node(&mut self, _document: &Document) -> Option<NodeId> {
        None
    }

    pub fn detach(&mut self) {}

    fn accepts_node(&self, node: NodeId) -> bool {
        if let Some(ref f) = self.filter {
            f(node)
        } else {
            true
        }
    }
}

impl std::fmt::Debug for NodeIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeIterator")
            .field("root", &self.root)
            .field("what_to_show", &self.what_to_show)
            .field("reference_node", &self.reference_node)
            .field(
                "pointer_before_reference_node",
                &self.pointer_before_reference_node,
            )
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct DOMRange {
    start_container: NodeId,
    start_offset: u32,
    end_container: NodeId,
    end_offset: u32,
    collapsed: bool,
    common_ancestor_container: Option<NodeId>,
}

impl Default for DOMRange {
    fn default() -> Self {
        Self::new()
    }
}

impl DOMRange {
    pub fn new() -> Self {
        Self {
            start_container: NodeId::new(),
            start_offset: 0,
            end_container: NodeId::new(),
            end_offset: 0,
            collapsed: true,
            common_ancestor_container: None,
        }
    }

    pub fn set_start(&mut self, node: NodeId, offset: u32) -> Result<()> {
        self.start_container = node;
        self.start_offset = offset;
        self.update_collapsed();
        Ok(())
    }

    pub fn set_end(&mut self, node: NodeId, offset: u32) -> Result<()> {
        self.end_container = node;
        self.end_offset = offset;
        self.update_collapsed();
        Ok(())
    }

    pub fn set_start_before(&mut self, node: NodeId, document: &Document) -> Result<()> {
        if let Some(parent) = document.get_parent(node) {
            let children = document.get_children(parent);
            if let Some(idx) = children.iter().position(|&id| id == node) {
                self.set_start(parent, idx as u32)?;
            }
        }
        Ok(())
    }

    pub fn set_start_after(&mut self, node: NodeId, document: &Document) -> Result<()> {
        if let Some(parent) = document.get_parent(node) {
            let children = document.get_children(parent);
            if let Some(idx) = children.iter().position(|&id| id == node) {
                self.set_start(parent, (idx + 1) as u32)?;
            }
        }
        Ok(())
    }

    pub fn set_end_before(&mut self, node: NodeId, document: &Document) -> Result<()> {
        if let Some(parent) = document.get_parent(node) {
            let children = document.get_children(parent);
            if let Some(idx) = children.iter().position(|&id| id == node) {
                self.set_end(parent, idx as u32)?;
            }
        }
        Ok(())
    }

    pub fn set_end_after(&mut self, node: NodeId, document: &Document) -> Result<()> {
        if let Some(parent) = document.get_parent(node) {
            let children = document.get_children(parent);
            if let Some(idx) = children.iter().position(|&id| id == node) {
                self.set_end(parent, (idx + 1) as u32)?;
            }
        }
        Ok(())
    }

    pub fn collapse(&mut self, to_start: bool) {
        if to_start {
            self.end_container = self.start_container;
            self.end_offset = self.start_offset;
        } else {
            self.start_container = self.end_container;
            self.start_offset = self.end_offset;
        }
        self.collapsed = true;
    }

    pub fn select_node(&mut self, node: NodeId, document: &Document) -> Result<()> {
        self.set_start_before(node, document)?;
        self.set_end_after(node, document)?;
        Ok(())
    }

    pub fn select_node_contents(&mut self, node: NodeId, document: &Document) -> Result<()> {
        let children = document.get_children(node);
        self.set_start(node, 0)?;
        self.set_end(node, children.len() as u32)?;
        Ok(())
    }

    pub fn delete_contents(&mut self, _document: &Document) -> Result<()> {
        Ok(())
    }

    pub fn extract_contents(&mut self, _document: &Document) -> Result<NodeId> {
        Ok(NodeId::new())
    }

    pub fn clone_contents(&self, _document: &Document) -> Result<NodeId> {
        Ok(NodeId::new())
    }

    pub fn insert_node(&mut self, _node: NodeId, _document: &Document) -> Result<()> {
        Ok(())
    }

    pub fn surround_contents(&mut self, _new_parent: NodeId, _document: &Document) -> Result<()> {
        Ok(())
    }

    pub fn clone_range(&self) -> Self {
        self.clone()
    }

    pub fn detach(&mut self) {}

    pub fn is_point_in_range(&self, _node: NodeId, _offset: u32, _document: &Document) -> bool {
        false
    }

    pub fn compare_point(&self, _node: NodeId, _offset: u32, _document: &Document) -> i32 {
        0
    }

    pub fn intersects_node(&self, _node: NodeId, _document: &Document) -> bool {
        false
    }

    fn update_collapsed(&mut self) {
        self.collapsed =
            self.start_container == self.end_container && self.start_offset == self.end_offset;
    }
}
