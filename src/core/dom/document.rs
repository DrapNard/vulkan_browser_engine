use std::sync::Arc;
use std::collections::HashMap;
use parking_lot::RwLock;
use dashmap::DashMap;
use smallvec::SmallVec;
use thiserror::Error;
use serde::{Serialize, Deserialize};

#[derive(Error, Debug)]
pub enum DocumentError {
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Node not found: {0}")]
    NodeNotFound(String),
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
    #[error("Memory allocation failed: {0}")]
    Memory(String),
    #[error("Query error: {0}")]
    Query(String),
}

pub type Result<T> = std::result::Result<T, DocumentError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub u64);

impl NodeId {
    pub fn new() -> Self {
        Self(fastrand::u64(..))
    }

    pub fn is_text(&self) -> bool {
        self.0 & 0x1 == 0x1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    Element,
    Text,
    Comment,
    Document,
    DocumentType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub title: String,
    pub url: Option<String>,
    pub charset: String,
    pub content_type: String,
    pub last_modified: Option<std::time::SystemTime>,
    pub ready_state: DocumentReadyState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentReadyState {
    Loading,
    Interactive,
    Complete,
}

impl Default for DocumentMetadata {
    fn default() -> Self {
        Self {
            title: String::new(),
            url: None,
            charset: "UTF-8".to_string(),
            content_type: "text/html".to_string(),
            last_modified: None,
            ready_state: DocumentReadyState::Loading,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MutationRecord {
    pub mutation_type: MutationType,
    pub target: NodeId,
    pub added_nodes: Vec<NodeId>,
    pub removed_nodes: Vec<NodeId>,
    pub previous_sibling: Option<NodeId>,
    pub next_sibling: Option<NodeId>,
    pub attribute_name: Option<String>,
    pub attribute_namespace: Option<String>,
    pub old_value: Option<String>,
    pub timestamp: std::time::Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationType {
    ChildList,
    Attributes,
    CharacterData,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub node_type: NodeType,
    pub tag_name: String,
    pub text_content: String,
    pub attributes: HashMap<String, String>,
    pub parent: Option<NodeId>,
    pub children: SmallVec<[NodeId; 8]>,
    pub namespace_uri: Option<String>,
}

impl Node {
    pub fn new_element(tag_name: String, id: NodeId) -> Self {
        Self {
            id,
            node_type: NodeType::Element,
            tag_name,
            text_content: String::new(),
            attributes: HashMap::new(),
            parent: None,
            children: SmallVec::new(),
            namespace_uri: None,
        }
    }

    pub fn new_text(content: String, id: NodeId) -> Self {
        Self {
            id,
            node_type: NodeType::Text,
            tag_name: "#text".to_string(),
            text_content: content,
            attributes: HashMap::new(),
            parent: None,
            children: SmallVec::new(),
            namespace_uri: None,
        }
    }

    pub fn new_comment(content: String, id: NodeId) -> Self {
        Self {
            id,
            node_type: NodeType::Comment,
            tag_name: "#comment".to_string(),
            text_content: content,
            attributes: HashMap::new(),
            parent: None,
            children: SmallVec::new(),
            namespace_uri: None,
        }
    }

    pub fn new_document(id: NodeId) -> Self {
        Self {
            id,
            node_type: NodeType::Document,
            tag_name: "#document".to_string(),
            text_content: String::new(),
            attributes: HashMap::new(),
            parent: None,
            children: SmallVec::new(),
            namespace_uri: None,
        }
    }

    pub fn new_doctype(name: String, id: NodeId) -> Self {
        Self {
            id,
            node_type: NodeType::DocumentType,
            tag_name: name,
            text_content: String::new(),
            attributes: HashMap::new(),
            parent: None,
            children: SmallVec::new(),
            namespace_uri: None,
        }
    }

    pub fn set_attribute(&mut self, name: &str, value: &str) {
        self.attributes.insert(name.to_string(), value.to_string());
    }

    pub fn get_attribute(&self, name: &str) -> Option<String> {
        self.attributes.get(name).cloned()
    }

    pub fn has_attribute(&self, name: &str) -> bool {
        self.attributes.contains_key(name)
    }

    pub fn get_tag_name(&self) -> &str {
        &self.tag_name
    }

    pub fn get_text_content(&self) -> String {
        self.text_content.clone()
    }

    pub fn reset(&mut self, node_type: NodeType, tag_name: String) {
        self.node_type = node_type;
        self.tag_name = tag_name;
        self.text_content.clear();
        self.attributes.clear();
        self.parent = None;
        self.children.clear();
        self.namespace_uri = None;
    }

    pub fn is_text(&self) -> bool {
        self.node_type == NodeType::Text
    }
}

#[derive(Debug, Clone)]
pub struct QueryCache {
    selector_cache: Arc<DashMap<String, Vec<NodeId>>>,
    id_cache: Arc<DashMap<String, NodeId>>,
    class_cache: Arc<DashMap<String, Vec<NodeId>>>,
    tag_cache: Arc<DashMap<String, Vec<NodeId>>>,
    cache_version: Arc<RwLock<u64>>,
}

impl QueryCache {
    pub fn new() -> Self {
        Self {
            selector_cache: Arc::new(DashMap::new()),
            id_cache: Arc::new(DashMap::new()),
            class_cache: Arc::new(DashMap::new()),
            tag_cache: Arc::new(DashMap::new()),
            cache_version: Arc::new(RwLock::new(0)),
        }
    }

    pub fn invalidate(&self) {
        let mut version = self.cache_version.write();
        *version += 1;
        self.selector_cache.clear();
        self.id_cache.clear();
        self.class_cache.clear();
        self.tag_cache.clear();
    }

    pub fn invalidate_partial(&self, _node_id: NodeId) {
        self.selector_cache.clear();
    }

    pub fn get_by_selector(&self, selector: &str) -> Option<Vec<NodeId>> {
        self.selector_cache.get(selector).map(|entry| entry.clone())
    }

    pub fn cache_selector_result(&self, selector: &str, result: Vec<NodeId>) {
        self.selector_cache.insert(selector.to_string(), result);
    }

    pub fn get_by_id(&self, id: &str) -> Option<NodeId> {
        self.id_cache.get(id).map(|entry| *entry.value())
    }

    pub fn cache_id(&self, id: &str, node_id: NodeId) {
        self.id_cache.insert(id.to_string(), node_id);
    }

    pub fn get_by_class(&self, class_name: &str) -> Option<Vec<NodeId>> {
        self.class_cache.get(class_name).map(|entry| entry.clone())
    }

    pub fn cache_class(&self, class_name: &str, node_ids: Vec<NodeId>) {
        self.class_cache.insert(class_name.to_string(), node_ids);
    }

    pub fn get_by_tag(&self, tag_name: &str) -> Option<Vec<NodeId>> {
        self.tag_cache.get(tag_name).map(|entry| entry.clone())
    }

    pub fn cache_tag(&self, tag_name: &str, node_ids: Vec<NodeId>) {
        self.tag_cache.insert(tag_name.to_string(), node_ids);
    }

    pub fn get_version(&self) -> u64 {
        *self.cache_version.read()
    }
}

#[derive(Clone)]
pub struct InlineScript {
    pub content: String,
    pub script_type: String,
    pub async_loading: bool,
    pub defer_execution: bool,
    pub integrity: Option<String>,
    pub nonce: Option<String>,
}

#[derive(Clone)]
pub struct MutationObserver {
    pub callback: Arc<dyn Fn(&[MutationRecord]) + Send + Sync>,
    pub observe_child_list: bool,
    pub observe_attributes: bool,
    pub observe_character_data: bool,
    pub observe_subtree: bool,
    pub attribute_filter: Option<Vec<String>>,
}

impl std::fmt::Debug for MutationObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MutationObserver")
            .field("observe_child_list", &self.observe_child_list)
            .field("observe_attributes", &self.observe_attributes)
            .field("observe_character_data", &self.observe_character_data)
            .field("observe_subtree", &self.observe_subtree)
            .field("attribute_filter", &self.attribute_filter)
            .finish()
    }
}

pub struct Document {
    metadata: Arc<RwLock<DocumentMetadata>>,
    root_node: Arc<RwLock<Option<NodeId>>>,
    nodes: Arc<DashMap<NodeId, Arc<RwLock<Node>>>>,
    query_cache: Arc<QueryCache>,
    mutation_observers: Arc<RwLock<Vec<MutationObserver>>>,
    mutation_records: Arc<RwLock<Vec<MutationRecord>>>,
}

impl Document {
    pub fn new() -> Self {
        Self {
            metadata: Arc::new(RwLock::new(DocumentMetadata::default())),
            root_node: Arc::new(RwLock::new(None)),
            nodes: Arc::new(DashMap::new()),
            query_cache: Arc::new(QueryCache::new()),
            mutation_observers: Arc::new(RwLock::new(Vec::new())),
            mutation_records: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn get_root_node(&self) -> Option<NodeId> {
        *self.root_node.read()
    }

    pub fn parse_html(&self, html: &str) -> Result<()> {
        let parse_start = std::time::Instant::now();
        self.query_cache.invalidate();
        let document_node_id = self.create_node(NodeType::Document, "".to_string())?;
        *self.root_node.write() = Some(document_node_id);
        let parser = HTMLParser::new();
        parser.parse(html, document_node_id, self)?;
        {
            let mut metadata = self.metadata.write();
            metadata.ready_state = DocumentReadyState::Interactive;
        }
        let parse_time = parse_start.elapsed();
        tracing::debug!("HTML parsing completed in {:?}", parse_time);
        Ok(())
    }

    pub fn parse(html: &str) -> Result<Self> {
        let document = Self::new();
        document.parse_html(html)?;
        Ok(document)
    }

    pub fn create_node(&self, node_type: NodeType, content: String) -> Result<NodeId> {
        let node_id = NodeId::new();
        let node = match node_type {
            NodeType::Element => Arc::new(RwLock::new(Node::new_element(content, node_id))),
            NodeType::Text => Arc::new(RwLock::new(Node::new_text(content, node_id))),
            NodeType::Comment => Arc::new(RwLock::new(Node::new_comment(content, node_id))),
            NodeType::Document => Arc::new(RwLock::new(Node::new_document(node_id))),
            NodeType::DocumentType => Arc::new(RwLock::new(Node::new_doctype(content, node_id))),
        };
        self.nodes.insert(node_id, node);
        Ok(node_id)
    }

    pub fn append_child(&self, parent_id: NodeId, child_id: NodeId) -> Result<()> {
        if let Some(parent_node) = self.nodes.get(&parent_id) {
            parent_node.write().children.push(child_id);
        }
        if let Some(child_node) = self.nodes.get(&child_id) {
            child_node.write().parent = Some(parent_id);
        }
        let record = MutationRecord {
            mutation_type: MutationType::ChildList,
            target: parent_id,
            added_nodes: vec![child_id],
            removed_nodes: Vec::new(),
            previous_sibling: None,
            next_sibling: None,
            attribute_name: None,
            attribute_namespace: None,
            old_value: None,
            timestamp: std::time::Instant::now(),
        };
        self.record_mutation(record);
        self.query_cache.invalidate_partial(parent_id);
        Ok(())
    }

    pub fn remove_child(&self, parent_id: NodeId, child_id: NodeId) -> Result<()> {
        if let Some(parent_node) = self.nodes.get(&parent_id) {
            parent_node.write().children.retain(|id| *id != child_id);
        }
        if let Some(child_node) = self.nodes.get(&child_id) {
            child_node.write().parent = None;
        }
        let record = MutationRecord {
            mutation_type: MutationType::ChildList,
            target: parent_id,
            added_nodes: Vec::new(),
            removed_nodes: vec![child_id],
            previous_sibling: None,
            next_sibling: None,
            attribute_name: None,
            attribute_namespace: None,
            old_value: None,
            timestamp: std::time::Instant::now(),
        };
        self.record_mutation(record);
        self.query_cache.invalidate_partial(parent_id);
        Ok(())
    }

    pub fn get_element_by_id(&self, id: &str) -> Option<NodeId> {
        if let Some(cached) = self.query_cache.get_by_id(id) {
            return Some(cached);
        }
        for entry in self.nodes.iter() {
            let node = entry.value().read();
            if let Some(attr) = node.get_attribute("id") {
                if attr == id {
                    let node_id = *entry.key();
                    self.query_cache.cache_id(id, node_id);
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn get_elements_by_class_name(&self, class_name: &str) -> Vec<NodeId> {
        if let Some(cached) = self.query_cache.get_by_class(class_name) {
            return cached;
        }
        let mut result = Vec::new();
        for entry in self.nodes.iter() {
            let node = entry.value().read();
            if let Some(classes) = node.get_attribute("class") {
                if classes.split_whitespace().any(|c| c == class_name) {
                    result.push(*entry.key());
                }
            }
        }
        self.query_cache.cache_class(class_name, result.clone());
        result
    }

    pub fn get_elements_by_tag_name(&self, tag_name: &str) -> Vec<NodeId> {
        if let Some(cached) = self.query_cache.get_by_tag(tag_name) {
            return cached;
        }
        let mut result = Vec::new();
        for entry in self.nodes.iter() {
            let node = entry.value().read();
            if node.get_tag_name().eq_ignore_ascii_case(tag_name) {
                result.push(*entry.key());
            }
        }
        self.query_cache.cache_tag(tag_name, result.clone());
        result
    }

    pub fn query_selector(&self, selector: &str) -> Result<Option<NodeId>> {
        let results = self.query_selector_all(selector)?;
        Ok(results.first().copied())
    }

    pub fn query_selector_all(&self, selector: &str) -> Result<Vec<NodeId>> {
        if let Some(cached) = self.query_cache.get_by_selector(selector) {
            return Ok(cached);
        }
        let result = self.execute_css_selector(selector)?;
        self.query_cache.cache_selector_result(selector, result.clone());
        Ok(result)
    }

    fn execute_css_selector(&self, selector: &str) -> Result<Vec<NodeId>> {
        let mut result = Vec::new();
        if selector.starts_with('#') {
            if let Some(id) = self.get_element_by_id(&selector[1..]) {
                result.push(id);
            }
        } else if selector.starts_with('.') {
            result = self.get_elements_by_class_name(&selector[1..]);
        } else if !selector.contains(' ') && !selector.contains('.') && !selector.contains('#') {
            result = self.get_elements_by_tag_name(selector);
        } else {
            result = self.complex_selector_matching(selector)?;
        }
        Ok(result)
    }

    fn complex_selector_matching(&self, _selector: &str) -> Result<Vec<NodeId>> {
        Ok(Vec::new())
    }

    pub fn get_inline_scripts(&self) -> Vec<InlineScript> {
        let mut scripts = Vec::new();
        for node_id in self.get_elements_by_tag_name("script") {
            if let Some(node_arc) = self.nodes.get(&node_id) {
                let node = node_arc.read();
                if node.get_attribute("src").is_some() {
                    continue;
                }
                let content = node.get_text_content();
                if content.trim().is_empty() {
                    continue;
                }
                scripts.push(InlineScript {
                    content,
                    script_type: node.get_attribute("type").unwrap_or_else(|| "text/javascript".to_string()),
                    async_loading: node.has_attribute("async"),
                    defer_execution: node.has_attribute("defer"),
                    integrity: node.get_attribute("integrity"),
                    nonce: node.get_attribute("nonce"),
                });
            }
        }
        scripts
    }

    pub fn get_node(&self, node_id: NodeId) -> Option<Arc<RwLock<Node>>> {
        self.nodes.get(&node_id).map(|e| e.clone())
    }

    pub fn get_children(&self, node_id: NodeId) -> Vec<NodeId> {
        if let Some(node) = self.nodes.get(&node_id) {
            node.read().children.iter().copied().collect()
        } else {
            Vec::new()
        }
    }

    pub fn get_parent(&self, node_id: NodeId) -> Option<NodeId> {
        if let Some(node) = self.nodes.get(&node_id) {
            node.read().parent
        } else {
            None
        }
    }

    pub fn get_url(&self) -> Option<String> {
        self.metadata.read().url.clone()
    }

    pub fn set_url(&self, url: String) {
        self.metadata.write().url = Some(url);
    }

    pub fn get_title(&self) -> String {
        self.metadata.read().title.clone()
    }

    pub fn set_title(&self, title: String) {
        self.metadata.write().title = title;
    }

    pub fn get_ready_state(&self) -> DocumentReadyState {
        self.metadata.read().ready_state
    }

    pub fn set_ready_state(&self, state: DocumentReadyState) {
        self.metadata.write().ready_state = state;
    }

    fn record_mutation(&self, record: MutationRecord) {
        self.mutation_records.write().push(record.clone());
        for observer in self.mutation_observers.read().iter() {
            (observer.callback)(&[record.clone()]);
        }
    }

    pub fn add_mutation_observer(&self, observer: MutationObserver) {
        self.mutation_observers.write().push(observer);
    }

    pub fn get_mutation_records(&self) -> Vec<MutationRecord> {
        let mut records = self.mutation_records.write();
        let result = records.clone();
        records.clear();
        result
    }

    pub fn get_performance_metrics(&self) -> serde_json::Value {
        serde_json::json!({
            "node_count": self.nodes.len(),
            "cache_version": self.query_cache.get_version(),
            "cache_entries": {
                "selector": self.query_cache.selector_cache.len(),
                "id": self.query_cache.id_cache.len(),
                "class": self.query_cache.class_cache.len(),
                "tag": self.query_cache.tag_cache.len()
            }
        })
    }

    pub async fn cleanup(&self) {
        self.query_cache.invalidate();
        self.mutation_records.write().clear();
        let all_node_ids: Vec<NodeId> = self.nodes.iter().map(|e| *e.key()).collect();
        for node_id in all_node_ids {
            self.nodes.remove(&node_id);
        }
    }
}

struct HTMLParser {
    current_node: Option<NodeId>,
}

impl HTMLParser {
    fn new() -> Self {
        Self { current_node: None }
    }

    fn parse(&self, _html: &str, _root_id: NodeId, _document: &Document) -> Result<()> {
        Ok(())
    }
}
