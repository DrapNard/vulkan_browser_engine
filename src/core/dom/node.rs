use std::sync::Arc;
use std::collections::HashMap;
use parking_lot::RwLock;
use smallvec::SmallVec;
use serde::{Serialize, Deserialize};

use super::document::NodeId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeType {
    Element = 1,
    Text = 3,
    Comment = 8,
    Document = 9,
    DocumentType = 10,
}

impl NodeType {
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            1 => Some(NodeType::Element),
            3 => Some(NodeType::Text),
            8 => Some(NodeType::Comment),
            9 => Some(NodeType::Document),
            10 => Some(NodeType::DocumentType),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AttributeMap {
    map: SmallVec<[(String, String); 8]>,
    namespace_map: HashMap<String, String>,
}

impl AttributeMap {
    pub fn new() -> Self {
        Self {
            map: SmallVec::new(),
            namespace_map: HashMap::new(),
        }
    }

    pub fn get(&self, name: &str) -> Option<&String> {
        self.map.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    pub fn set(&mut self, name: String, value: String) {
        if let Some((_, v)) = self.map.iter_mut().find(|(k, _)| k == &name) {
            *v = value;
        } else {
            self.map.push((name, value));
        }
    }

    pub fn remove(&mut self, name: &str) -> Option<String> {
        if let Some(pos) = self.map.iter().position(|(k, _)| k == name) {
            Some(self.map.remove(pos).1)
        } else {
            None
        }
    }

    pub fn has(&self, name: &str) -> bool {
        self.map.iter().any(|(k, _)| k == name)
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.map.iter().map(|(k, _)| k)
    }

    pub fn entries(&self) -> impl Iterator<Item = (&String, &String)> {
        self.map.iter().map(|(k, v)| (k, v))
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.namespace_map.clear();
    }

    pub fn set_namespaced(&mut self, namespace: String, name: String, value: String) {
        let full = format!("{}:{}", namespace, name);
        self.namespace_map.insert(full.clone(), namespace);
        self.set(full, value);
    }

    pub fn get_namespaced(&self, namespace: &str, name: &str) -> Option<&String> {
        let full = format!("{}:{}", namespace, name);
        self.get(&full)
    }
}

#[derive(Debug, Clone)]
pub struct ComputedStyle {
    properties: SmallVec<[(String, String); 16]>,
    cascaded_properties: HashMap<String, Vec<(String, u32)>>,
    inherited_properties: SmallVec<[String; 8]>,
    is_dirty: bool,
    parent_style: Option<Arc<ComputedStyle>>,
}

impl ComputedStyle {
    pub fn new() -> Self {
        Self {
            properties: SmallVec::new(),
            cascaded_properties: HashMap::new(),
            inherited_properties: SmallVec::new(),
            is_dirty: true,
            parent_style: None,
        }
    }

    pub fn get_property(&self, name: &str) -> Option<&String> {
        self.properties.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    pub fn set_property(&mut self, name: String, value: String) {
        if let Some((_, v)) = self.properties.iter_mut().find(|(k, _)| k == &name) {
            *v = value;
        } else {
            self.properties.push((name, value));
        }
        self.is_dirty = true;
    }

    pub fn inherit_from(&mut self, parent: Arc<ComputedStyle>) {
        self.parent_style = Some(parent.clone());
        let inherited = self.inherited_properties.clone();
        for property in inherited {
            if let Some(value) = parent.get_property(&property) {
                self.set_property(property.clone(), value.clone());
            }
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.is_dirty
    }

    pub fn mark_clean(&mut self) {
        self.is_dirty = false;
    }

    pub fn mark_dirty(&mut self) {
        self.is_dirty = true;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayType {
    None,
    Block,
    Inline,
    InlineBlock,
    Flex,
    Grid,
    Table,
    TableRow,
    TableCell,
    ListItem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionType {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatType {
    None,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearType {
    None,
    Left,
    Right,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowType {
    Visible,
    Hidden,
    Scroll,
    Auto,
}

#[derive(Debug, Clone)]
pub struct LayoutData {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub margin_top: f32,
    pub margin_right: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub padding_top: f32,
    pub padding_right: f32,
    pub padding_bottom: f32,
    pub padding_left: f32,
    pub border_top: f32,
    pub border_right: f32,
    pub border_bottom: f32,
    pub border_left: f32,
    pub content_width: f32,
    pub content_height: f32,
    pub is_positioned: bool,
    pub z_index: i32,
    pub display: DisplayType,
    pub position: PositionType,
    pub float: FloatType,
    pub clear: ClearType,
    pub overflow_x: OverflowType,
    pub overflow_y: OverflowType,
    pub is_dirty: bool,
}

impl Default for LayoutData {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            margin_top: 0.0,
            margin_right: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            border_top: 0.0,
            border_right: 0.0,
            border_bottom: 0.0,
            border_left: 0.0,
            content_width: 0.0,
            content_height: 0.0,
            is_positioned: false,
            z_index: 0,
            display: DisplayType::Block,
            position: PositionType::Static,
            float: FloatType::None,
            clear: ClearType::None,
            overflow_x: OverflowType::Visible,
            overflow_y: OverflowType::Visible,
            is_dirty: true,
        }
    }
}

impl LayoutData {
    pub fn get_margin_box_width(&self) -> f32 {
        self.width + self.margin_left + self.margin_right
    }

    pub fn get_margin_box_height(&self) -> f32 {
        self.height + self.margin_top + self.margin_bottom
    }

    pub fn get_border_box_width(&self) -> f32 {
        self.width
    }

    pub fn get_border_box_height(&self) -> f32 {
        self.height
    }

    pub fn get_padding_box_width(&self) -> f32 {
        self.width - self.border_left - self.border_right
    }

    pub fn get_padding_box_height(&self) -> f32 {
        self.height - self.border_top - self.border_bottom
    }

    pub fn get_content_box_width(&self) -> f32 {
        self.content_width
    }

    pub fn get_content_box_height(&self) -> f32 {
        self.content_height
    }

    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.width &&
        y >= self.y && y <= self.y + self.height
    }

    pub fn intersects(&self, other: &LayoutData) -> bool {
        !(self.x + self.width < other.x ||
          other.x + other.width < self.x ||
          self.y + self.height < other.y ||
          other.y + other.height < self.y)
    }

    pub fn mark_dirty(&mut self) {
        self.is_dirty = true;
    }

    pub fn mark_clean(&mut self) {
        self.is_dirty = false;
    }
}

pub struct Node {
    node_id: NodeId,
    node_type: NodeType,
    tag_name: String,
    text_content: String,
    attributes: AttributeMap,
    computed_style: Arc<RwLock<ComputedStyle>>,
    layout_data: Arc<RwLock<LayoutData>>,
    event_listeners: HashMap<String, Vec<Arc<dyn Fn() + Send + Sync>>>,
    custom_data: HashMap<String, Box<dyn std::any::Any + Send + Sync>>,
    namespace_uri: Option<String>,
    prefix: Option<String>,
    base_uri: Option<String>,
    owner_document: Option<NodeId>,
    is_connected: bool,
    shadow_root: Option<NodeId>,
    assigned_slot: Option<NodeId>,
}

impl Node {
    pub fn new_element(tag_name: String, node_id: NodeId) -> Self {
        Self {
            node_id,
            node_type: NodeType::Element,
            tag_name: tag_name.to_lowercase(),
            text_content: String::new(),
            attributes: AttributeMap::new(),
            computed_style: Arc::new(RwLock::new(ComputedStyle::new())),
            layout_data: Arc::new(RwLock::new(LayoutData::default())),
            event_listeners: HashMap::new(),
            custom_data: HashMap::new(),
            namespace_uri: None,
            prefix: None,
            base_uri: None,
            owner_document: None,
            is_connected: false,
            shadow_root: None,
            assigned_slot: None,
        }
    }

    pub fn new_text(content: String, node_id: NodeId) -> Self {
        Self {
            node_id,
            node_type: NodeType::Text,
            tag_name: String::new(),
            text_content: content,
            attributes: AttributeMap::new(),
            computed_style: Arc::new(RwLock::new(ComputedStyle::new())),
            layout_data: Arc::new(RwLock::new(LayoutData::default())),
            event_listeners: HashMap::new(),
            custom_data: HashMap::new(),
            namespace_uri: None,
            prefix: None,
            base_uri: None,
            owner_document: None,
            is_connected: false,
            shadow_root: None,
            assigned_slot: None,
        }
    }

    pub fn new_comment(content: String, node_id: NodeId) -> Self {
        Self {
            node_id,
            node_type: NodeType::Comment,
            tag_name: String::new(),
            text_content: content,
            attributes: AttributeMap::new(),
            computed_style: Arc::new(RwLock::new(ComputedStyle::new())),
            layout_data: Arc::new(RwLock::new(LayoutData::default())),
            event_listeners: HashMap::new(),
            custom_data: HashMap::new(),
            namespace_uri: None,
            prefix: None,
            base_uri: None,
            owner_document: None,
            is_connected: false,
            shadow_root: None,
            assigned_slot: None,
        }
    }

    pub fn new_document(node_id: NodeId) -> Self {
        Self {
            node_id,
            node_type: NodeType::Document,
            tag_name: String::new(),
            text_content: String::new(),
            attributes: AttributeMap::new(),
            computed_style: Arc::new(RwLock::new(ComputedStyle::new())),
            layout_data: Arc::new(RwLock::new(LayoutData::default())),
            event_listeners: HashMap::new(),
            custom_data: HashMap::new(),
            namespace_uri: None,
            prefix: None,
            base_uri: None,
            owner_document: None,
            is_connected: true,
            shadow_root: None,
            assigned_slot: None,
        }
    }

    pub fn new_doctype(name: String, node_id: NodeId) -> Self {
        Self {
            node_id,
            node_type: NodeType::DocumentType,
            tag_name: name,
            text_content: String::new(),
            attributes: AttributeMap::new(),
            computed_style: Arc::new(RwLock::new(ComputedStyle::new())),
            layout_data: Arc::new(RwLock::new(LayoutData::default())),
            event_listeners: HashMap::new(),
            custom_data: HashMap::new(),
            namespace_uri: None,
            prefix: None,
            base_uri: None,
            owner_document: None,
            is_connected: false,
            shadow_root: None,
            assigned_slot: None,
        }
    }

    pub fn reset(&mut self, node_type: NodeType, tag_name: String) {
        self.node_type = node_type;
        self.tag_name = tag_name;
        self.text_content.clear();
        self.attributes.clear();
        self.computed_style.write().mark_dirty();
        self.layout_data.write().mark_dirty();
        self.event_listeners.clear();
        self.custom_data.clear();
        self.namespace_uri = None;
        self.prefix = None;
        self.base_uri = None;
        self.owner_document = None;
        self.is_connected = false;
        self.shadow_root = None;
        self.assigned_slot = None;
    }

    pub fn get_node_id(&self) -> NodeId {
        self.node_id
    }

    pub fn get_node_type(&self) -> NodeType {
        self.node_type
    }

    pub fn get_tag_name(&self) -> &str {
        &self.tag_name
    }

    pub fn set_tag_name(&mut self, tag_name: String) {
        self.tag_name = tag_name.to_lowercase();
    }

    pub fn get_text_content(&self) -> String {
        self.text_content.clone()
    }

    pub fn set_text_content(&mut self, content: String) {
        self.text_content = content;
    }

    pub fn get_attribute(&self, name: &str) -> Option<String> {
        self.attributes.get(name).cloned()
    }

    pub fn set_attribute(&mut self, name: &str, value: &str) {
        self.attributes.set(name.to_string(), value.to_string());
        if self.node_type == NodeType::Element {
            self.invalidate_style();
        }
    }

    pub fn remove_attribute(&mut self, name: &str) -> Option<String> {
        let r = self.attributes.remove(name);
        if r.is_some() && self.node_type == NodeType::Element {
            self.invalidate_style();
        }
        r
    }

    pub fn has_attribute(&self, name: &str) -> bool {
        self.attributes.has(name)
    }

    pub fn get_attribute_names(&self) -> Vec<String> {
        self.attributes.keys().cloned().collect()
    }

    pub fn get_attributes(&self) -> impl Iterator<Item = (&String, &String)> {
        self.attributes.entries()
    }

    pub fn get_computed_style(&self) -> Arc<RwLock<ComputedStyle>> {
        self.computed_style.clone()
    }

    pub fn get_layout_data(&self) -> Arc<RwLock<LayoutData>> {
        self.layout_data.clone()
    }

    pub fn invalidate_style(&self) {
        self.computed_style.write().mark_dirty();
    }

    pub fn invalidate_layout(&self) {
        self.layout_data.write().mark_dirty();
    }

    pub fn add_event_listener<F>(&mut self, event: String, listener: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.event_listeners
            .entry(event)
            .or_insert_with(Vec::new)
            .push(Arc::new(listener));
    }

    pub fn remove_event_listeners(&mut self, event: &str) {
        self.event_listeners.remove(event);
    }

    pub fn dispatch_event(&self, event: &str) {
        if let Some(list) = self.event_listeners.get(event) {
            for listener in list {
                listener();
            }
        }
    }

    pub fn get_namespace_uri(&self) -> Option<&String> {
        self.namespace_uri.as_ref()
    }

    pub fn set_namespace_uri(&mut self, ns: Option<String>) {
        self.namespace_uri = ns;
    }

    pub fn get_prefix(&self) -> Option<&String> {
        self.prefix.as_ref()
    }

    pub fn set_prefix(&mut self, p: Option<String>) {
        self.prefix = p;
    }

    pub fn get_base_uri(&self) -> Option<&String> {
        self.base_uri.as_ref()
    }

    pub fn set_base_uri(&mut self, b: Option<String>) {
        self.base_uri = b;
    }

    pub fn get_owner_document(&self) -> Option<NodeId> {
        self.owner_document
    }

    pub fn set_owner_document(&mut self, doc: Option<NodeId>) {
        self.owner_document = doc;
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected
    }

    pub fn set_connected(&mut self, c: bool) {
        self.is_connected = c;
    }

    pub fn get_shadow_root(&self) -> Option<NodeId> {
        self.shadow_root
    }

    pub fn set_shadow_root(&mut self, root: Option<NodeId>) {
        self.shadow_root = root;
    }

    pub fn get_assigned_slot(&self) -> Option<NodeId> {
        self.assigned_slot
    }

    pub fn set_assigned_slot(&mut self, slot: Option<NodeId>) {
        self.assigned_slot = slot;
    }

    pub fn is_element(&self) -> bool {
        self.node_type == NodeType::Element
    }

    pub fn is_text(&self) -> bool {
        self.node_type == NodeType::Text
    }

    pub fn is_comment(&self) -> bool {
        self.node_type == NodeType::Comment
    }

    pub fn is_document(&self) -> bool {
        self.node_type == NodeType::Document
    }

    pub fn is_document_type(&self) -> bool {
        self.node_type == NodeType::DocumentType
    }

    pub fn matches_tag(&self, tag: &str) -> bool {
        self.tag_name.eq_ignore_ascii_case(tag)
    }

    pub fn matches_id(&self, id: &str) -> bool {
        self.get_attribute("id").map(|v| v == id).unwrap_or(false)
    }

    pub fn matches_class(&self, class: &str) -> bool {
        self.get_attribute("class")
            .map(|c| c.split_whitespace().any(|s| s == class))
            .unwrap_or(false)
    }

    pub fn get_classes(&self) -> Vec<String> {
        self.get_attribute("class")
            .map(|c| c.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default()
    }

    pub fn add_class(&mut self, class: &str) {
        let mut list = self.get_classes();
        if !list.contains(&class.to_string()) {
            list.push(class.to_string());
            self.set_attribute("class", &list.join(" "));
        }
    }

    pub fn remove_class(&mut self, class: &str) {
        let mut list = self.get_classes();
        list.retain(|c| c != class);
        if list.is_empty() {
            self.remove_attribute("class");
        } else {
            self.set_attribute("class", &list.join(" "));
        }
    }

    pub fn toggle_class(&mut self, class: &str) -> bool {
        if self.matches_class(class) {
            self.remove_class(class);
            false
        } else {
            self.add_class(class);
            true
        }
    }

    pub fn set_custom_data<T: std::any::Any + Send + Sync>(&mut self, key: String, data: T) {
        self.custom_data.insert(key, Box::new(data));
    }

    pub fn get_custom_data<T: std::any::Any + Send + Sync>(&self, key: &str) -> Option<&T> {
        self.custom_data.get(key).and_then(|b| b.downcast_ref::<T>())
    }

    pub fn remove_custom_data(&mut self, key: &str) -> Option<Box<dyn std::any::Any + Send + Sync>> {
        self.custom_data.remove(key)
    }

    pub fn clone_node(&self, deep: bool) -> Self {
        let cloned = Self {
            node_id: NodeId::new(),
            node_type: self.node_type,
            tag_name: self.tag_name.clone(),
            text_content: self.text_content.clone(),
            attributes: self.attributes.clone(),
            computed_style: Arc::new(RwLock::new(ComputedStyle::new())),
            layout_data: Arc::new(RwLock::new(LayoutData::default())),
            event_listeners: if deep { Vec::new().into_iter().collect() } else { self.event_listeners.clone() },
            custom_data: HashMap::new(),
            namespace_uri: self.namespace_uri.clone(),
            prefix: self.prefix.clone(),
            base_uri: self.base_uri.clone(),
            owner_document: self.owner_document,
            is_connected: false,
            shadow_root: None,
            assigned_slot: None,
        };
        cloned
    }

    pub fn get_memory_usage(&self) -> usize {
        let mut size = std::mem::size_of::<Self>();
        size += self.tag_name.capacity();
        size += self.text_content.capacity();
        size += self.attributes.map.capacity() * std::mem::size_of::<(String, String)>();
        size += self.attributes.namespace_map.capacity() * std::mem::size_of::<(String, String)>();
        size += self.event_listeners.capacity() * std::mem::size_of::<Vec<Arc<dyn Fn() + Send + Sync>>>();
        size += self.custom_data.capacity() * std::mem::size_of::<Box<dyn std::any::Any + Send + Sync>>();
        for (k, v) in &self.attributes.map {
            size += k.capacity() + v.capacity();
        }
        for k in self.custom_data.keys() {
            size += k.capacity();
        }
        size
    }
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("node_id", &self.node_id)
            .field("node_type", &self.node_type)
            .field("tag_name", &self.tag_name)
            .field("text_content", &self.text_content)
            .field("attributes", &self.attributes)
            .field("computed_style", &"<ComputedStyle>")
            .field("layout_data", &"<LayoutData>")
            .field("event_listeners_count", &self.event_listeners.len())
            .field("custom_data_keys", &self.custom_data.keys().collect::<Vec<_>>())
            .field("namespace_uri", &self.namespace_uri)
            .field("prefix", &self.prefix)
            .field("base_uri", &self.base_uri)
            .field("owner_document", &self.owner_document)
            .field("is_connected", &self.is_connected)
            .field("shadow_root", &self.shadow_root)
            .field("assigned_slot", &self.assigned_slot)
            .finish()
    }
}
