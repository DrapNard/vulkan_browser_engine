use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

use crate::core::css::CSSStyleDeclaration;
use crate::core::dom::document::NodeId;
use crate::core::dom::node::{Node, NodeType};

#[derive(Error, Debug)]
pub enum ElementError {
    #[error("Invalid element operation: {0}")]
    InvalidOperation(String),
    #[error("Property not found: {0}")]
    PropertyNotFound(String),
    #[error("Invalid property value: {0}")]
    InvalidPropertyValue(String),
    #[error("Selector error: {0}")]
    Selector(String),
}

pub type Result<T> = std::result::Result<T, ElementError>;

#[derive(Debug, Clone)]
pub struct DOMRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl DOMRect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
            top: y,
            right: x + width,
            bottom: y + height,
            left: x,
        }
    }

    pub fn from_layout_data(layout: &crate::core::dom::node::LayoutData) -> Self {
        Self::new(
            layout.x as f64,
            layout.y as f64,
            layout.width as f64,
            layout.height as f64,
        )
    }
}

#[derive(Debug, Clone)]
pub struct ElementProperties {
    pub inner_html: String,
    pub outer_html: String,
    pub text_content: String,
    pub inner_text: String,
    pub client_width: f64,
    pub client_height: f64,
    pub client_top: f64,
    pub client_left: f64,
    pub scroll_width: f64,
    pub scroll_height: f64,
    pub scroll_top: f64,
    pub scroll_left: f64,
    pub offset_width: f64,
    pub offset_height: f64,
    pub offset_top: f64,
    pub offset_left: f64,
    pub tab_index: i32,
    pub hidden: bool,
    pub content_editable: String,
    pub is_content_editable: bool,
    pub spellcheck: bool,
    pub translate: bool,
    pub dir: String,
    pub lang: String,
    pub title: String,
    pub access_key: String,
    pub draggable: bool,
    pub dropzone: String,
}

impl Default for ElementProperties {
    fn default() -> Self {
        Self {
            inner_html: String::new(),
            outer_html: String::new(),
            text_content: String::new(),
            inner_text: String::new(),
            client_width: 0.0,
            client_height: 0.0,
            client_top: 0.0,
            client_left: 0.0,
            scroll_width: 0.0,
            scroll_height: 0.0,
            scroll_top: 0.0,
            scroll_left: 0.0,
            offset_width: 0.0,
            offset_height: 0.0,
            offset_top: 0.0,
            offset_left: 0.0,
            tab_index: -1,
            hidden: false,
            content_editable: "inherit".to_string(),
            is_content_editable: false,
            spellcheck: true,
            translate: true,
            dir: String::new(),
            lang: String::new(),
            title: String::new(),
            access_key: String::new(),
            draggable: false,
            dropzone: String::new(),
        }
    }
}

#[derive(Clone, Default)]
pub struct ElementInternals {
    pub form_associated: bool,
    pub form_disabled: bool,
    pub form_reset_callback: Option<Arc<dyn Fn() + Send + Sync>>,
    pub form_state_restore_callback: Option<Arc<dyn Fn() + Send + Sync>>,
    pub validation_message: String,
    pub validity_state: ValidityState,
    pub will_validate: bool,
    pub labels: Vec<NodeId>,
    pub form: Option<NodeId>,
}

impl std::fmt::Debug for ElementInternals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ElementInternals")
            .field("form_associated", &self.form_associated)
            .field("form_disabled", &self.form_disabled)
            .field("validation_message", &self.validation_message)
            .field("validity_state", &self.validity_state)
            .field("will_validate", &self.will_validate)
            .field("labels", &self.labels)
            .field("form", &self.form)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct ValidityState {
    pub value_missing: bool,
    pub type_mismatch: bool,
    pub pattern_mismatch: bool,
    pub too_long: bool,
    pub too_short: bool,
    pub range_underflow: bool,
    pub range_overflow: bool,
    pub step_mismatch: bool,
    pub bad_input: bool,
    pub custom_error: bool,
    pub valid: bool,
}

impl Default for ValidityState {
    fn default() -> Self {
        Self {
            value_missing: false,
            type_mismatch: false,
            pattern_mismatch: false,
            too_long: false,
            too_short: false,
            range_underflow: false,
            range_overflow: false,
            step_mismatch: false,
            bad_input: false,
            custom_error: false,
            valid: true,
        }
    }
}

pub struct Element {
    node: Node,
    properties: Arc<RwLock<ElementProperties>>,
    style: Arc<RwLock<CSSStyleDeclaration>>,
    dataset: Arc<RwLock<HashMap<String, String>>>,
    class_list: Arc<RwLock<Vec<String>>>,
    internals: Arc<RwLock<ElementInternals>>,
    pseudo_elements: Arc<RwLock<HashMap<String, NodeId>>>,
    animation_properties: Arc<RwLock<AnimationProperties>>,
}

#[derive(Debug, Clone)]
pub struct AnimationProperties {
    pub animations: Vec<Animation>,
    pub transitions: Vec<Transition>,
    pub transform: String,
    pub opacity: f64,
    pub filter: String,
    pub backdrop_filter: String,
}

#[derive(Debug, Clone)]
pub struct Animation {
    pub name: String,
    pub duration: f64,
    pub timing_function: String,
    pub delay: f64,
    pub iteration_count: f64,
    pub direction: String,
    pub fill_mode: String,
    pub play_state: String,
}

#[derive(Debug, Clone)]
pub struct Transition {
    pub property: String,
    pub duration: f64,
    pub timing_function: String,
    pub delay: f64,
}

impl Default for AnimationProperties {
    fn default() -> Self {
        Self {
            animations: Vec::new(),
            transitions: Vec::new(),
            transform: String::new(),
            opacity: 1.0,
            filter: String::new(),
            backdrop_filter: String::new(),
        }
    }
}

impl Element {
    pub fn new(tag_name: String, node_id: NodeId) -> Self {
        Self {
            node: Node::new_element(tag_name, node_id),
            properties: Arc::new(RwLock::new(ElementProperties::default())),
            style: Arc::new(RwLock::new(CSSStyleDeclaration::new())),
            dataset: Arc::new(RwLock::new(HashMap::new())),
            class_list: Arc::new(RwLock::new(Vec::new())),
            internals: Arc::new(RwLock::new(ElementInternals::default())),
            pseudo_elements: Arc::new(RwLock::new(HashMap::new())),
            animation_properties: Arc::new(RwLock::new(AnimationProperties::default())),
        }
    }

    pub fn from_node(node: Node) -> Result<Self> {
        if node.get_node_type() != NodeType::Element {
            return Err(ElementError::InvalidOperation(
                "Node is not an element".to_string(),
            ));
        }
        let classes = node.get_classes();
        Ok(Self {
            node,
            properties: Arc::new(RwLock::new(ElementProperties::default())),
            style: Arc::new(RwLock::new(CSSStyleDeclaration::new())),
            dataset: Arc::new(RwLock::new(HashMap::new())),
            class_list: Arc::new(RwLock::new(classes)),
            internals: Arc::new(RwLock::new(ElementInternals::default())),
            pseudo_elements: Arc::new(RwLock::new(HashMap::new())),
            animation_properties: Arc::new(RwLock::new(AnimationProperties::default())),
        })
    }

    pub fn as_node(&self) -> &Node {
        &self.node
    }

    pub fn as_node_mut(&mut self) -> &mut Node {
        &mut self.node
    }

    pub fn get_id(&self) -> Option<String> {
        self.node.get_attribute("id")
    }

    pub fn set_id(&mut self, id: &str) {
        self.node.set_attribute("id", id);
    }

    pub fn get_class_name(&self) -> String {
        self.node.get_attribute("class").unwrap_or_default()
    }

    pub fn set_class_name(&mut self, class_name: &str) {
        self.node.set_attribute("class", class_name);
        let mut class_list = self.class_list.write();
        *class_list = class_name.split_whitespace().map(str::to_string).collect();
    }

    pub fn get_class_list(&self) -> Vec<String> {
        self.class_list.read().clone()
    }

    pub fn add_class(&mut self, class_name: &str) {
        self.node.add_class(class_name);
        let mut class_list = self.class_list.write();
        if !class_list.contains(&class_name.to_string()) {
            class_list.push(class_name.to_string());
        }
    }

    pub fn remove_class(&mut self, class_name: &str) {
        self.node.remove_class(class_name);
        let mut class_list = self.class_list.write();
        class_list.retain(|class| class != class_name);
    }

    pub fn toggle_class(&mut self, class_name: &str) -> bool {
        let result = self.node.toggle_class(class_name);
        let mut class_list = self.class_list.write();
        if result {
            if !class_list.contains(&class_name.to_string()) {
                class_list.push(class_name.to_string());
            }
        } else {
            class_list.retain(|class| class != class_name);
        }
        result
    }

    pub fn contains_class(&self, class_name: &str) -> bool {
        self.class_list.read().contains(&class_name.to_string())
    }

    pub fn get_style(&self) -> Arc<RwLock<CSSStyleDeclaration>> {
        self.style.clone()
    }

    pub fn set_style_property(&self, property: &str, value: &str) -> Result<()> {
        let style = self.style.write();
        style
            .set_property(property, value, "")
            .map_err(|e| ElementError::InvalidPropertyValue(e.to_string()))?;
        Ok(())
    }

    pub fn get_style_property(&self, property: &str) -> Option<String> {
        let style = self.style.read();
        style.get_property_value(property)
    }

    pub fn remove_style_property(&self, property: &str) -> Option<String> {
        let style = self.style.write();
        style.remove_property(property).ok()
    }

    pub fn get_inner_html(&self) -> String {
        self.properties.read().inner_html.clone()
    }

    pub fn set_inner_html(&mut self, html: &str) -> Result<()> {
        self.properties.write().inner_html = html.to_string();
        Ok(())
    }

    pub fn get_outer_html(&self) -> String {
        self.properties.read().outer_html.clone()
    }

    pub fn set_outer_html(&mut self, html: &str) -> Result<()> {
        self.properties.write().outer_html = html.to_string();
        Ok(())
    }

    pub fn get_text_content(&self) -> String {
        self.node.get_text_content()
    }

    pub fn set_text_content(&mut self, text: &str) {
        self.node.set_text_content(text.to_string());
        self.properties.write().text_content = text.to_string();
    }

    pub fn get_inner_text(&self) -> String {
        self.properties.read().inner_text.clone()
    }

    pub fn set_inner_text(&mut self, text: &str) {
        self.properties.write().inner_text = text.to_string();
    }

    pub fn get_bounding_client_rect(&self) -> DOMRect {
        let layout = self.node.get_layout_data();
        let layout_data = layout.read();
        DOMRect::from_layout_data(&layout_data)
    }

    pub fn get_client_rects(&self) -> Vec<DOMRect> {
        vec![self.get_bounding_client_rect()]
    }

    pub fn get_client_width(&self) -> f64 {
        self.properties.read().client_width
    }

    pub fn get_client_height(&self) -> f64 {
        self.properties.read().client_height
    }

    pub fn get_scroll_width(&self) -> f64 {
        self.properties.read().scroll_width
    }

    pub fn get_scroll_height(&self) -> f64 {
        self.properties.read().scroll_height
    }

    pub fn get_scroll_top(&self) -> f64 {
        self.properties.read().scroll_top
    }

    pub fn set_scroll_top(&self, top: f64) {
        self.properties.write().scroll_top = top.max(0.0);
    }

    pub fn get_scroll_left(&self) -> f64 {
        self.properties.read().scroll_left
    }

    pub fn set_scroll_left(&self, left: f64) {
        self.properties.write().scroll_left = left.max(0.0);
    }

    pub fn get_offset_width(&self) -> f64 {
        self.properties.read().offset_width
    }

    pub fn get_offset_height(&self) -> f64 {
        self.properties.read().offset_height
    }

    pub fn get_offset_top(&self) -> f64 {
        self.properties.read().offset_top
    }

    pub fn get_offset_left(&self) -> f64 {
        self.properties.read().offset_left
    }

    pub fn scroll_into_view(&self, align_to_top: bool) {
        let layout = self.node.get_layout_data();
        let layout_data = layout.read();
        if align_to_top {
            self.set_scroll_top(layout_data.y as f64);
        } else {
            self.set_scroll_top((layout_data.y + layout_data.height) as f64);
        }
    }

    pub fn scroll_by(&self, x: f64, y: f64) {
        let current_left = self.get_scroll_left();
        let current_top = self.get_scroll_top();
        self.set_scroll_left(current_left + x);
        self.set_scroll_top(current_top + y);
    }

    pub fn scroll_to(&self, x: f64, y: f64) {
        self.set_scroll_left(x);
        self.set_scroll_top(y);
    }

    pub fn get_tab_index(&self) -> i32 {
        self.properties.read().tab_index
    }

    pub fn set_tab_index(&mut self, index: i32) {
        self.properties.write().tab_index = index;
        self.node.set_attribute("tabindex", &index.to_string());
    }

    pub fn focus(&self) {}

    pub fn blur(&self) {}

    pub fn click(&mut self) {
        self.node.dispatch_event("click");
    }

    pub fn get_hidden(&self) -> bool {
        self.properties.read().hidden
    }

    pub fn set_hidden(&mut self, hidden: bool) {
        self.properties.write().hidden = hidden;
        if hidden {
            self.node.set_attribute("hidden", "");
        } else {
            self.node.remove_attribute("hidden");
        }
    }

    pub fn get_draggable(&self) -> bool {
        self.properties.read().draggable
    }

    pub fn set_draggable(&mut self, draggable: bool) {
        self.properties.write().draggable = draggable;
        self.node.set_attribute("draggable", &draggable.to_string());
    }

    pub fn get_content_editable(&self) -> String {
        self.properties.read().content_editable.clone()
    }

    pub fn set_content_editable(&mut self, editable: &str) {
        let mut props = self.properties.write();
        props.content_editable = editable.to_string();
        props.is_content_editable = editable == "true";
        self.node.set_attribute("contenteditable", editable);
    }

    pub fn is_content_editable(&self) -> bool {
        self.properties.read().is_content_editable
    }

    pub fn get_spellcheck(&self) -> bool {
        self.properties.read().spellcheck
    }

    pub fn set_spellcheck(&mut self, spellcheck: bool) {
        self.properties.write().spellcheck = spellcheck;
        self.node
            .set_attribute("spellcheck", &spellcheck.to_string());
    }

    pub fn get_title(&self) -> String {
        self.properties.read().title.clone()
    }

    pub fn set_title(&mut self, title: &str) {
        self.properties.write().title = title.to_string();
        self.node.set_attribute("title", title);
    }

    pub fn get_lang(&self) -> String {
        self.properties.read().lang.clone()
    }

    pub fn set_lang(&mut self, lang: &str) {
        self.properties.write().lang = lang.to_string();
        self.node.set_attribute("lang", lang);
    }

    pub fn get_dir(&self) -> String {
        self.properties.read().dir.clone()
    }

    pub fn set_dir(&mut self, dir: &str) {
        self.properties.write().dir = dir.to_string();
        self.node.set_attribute("dir", dir);
    }

    pub fn get_access_key(&self) -> String {
        self.properties.read().access_key.clone()
    }

    pub fn set_access_key(&mut self, key: &str) {
        self.properties.write().access_key = key.to_string();
        self.node.set_attribute("accesskey", key);
    }

    pub fn get_dataset(&self) -> HashMap<String, String> {
        self.dataset.read().clone()
    }

    pub fn set_dataset_value(&mut self, key: &str, value: &str) {
        self.dataset
            .write()
            .insert(key.to_string(), value.to_string());
        let attr_name = format!("data-{}", key.replace('_', "-"));
        self.node.set_attribute(&attr_name, value);
    }

    pub fn get_dataset_value(&self, key: &str) -> Option<String> {
        self.dataset.read().get(key).cloned()
    }

    pub fn remove_dataset_value(&mut self, key: &str) -> Option<String> {
        let result = self.dataset.write().remove(key);
        let attr_name = format!("data-{}", key.replace('_', "-"));
        self.node.remove_attribute(&attr_name);
        result
    }

    pub fn insert_adjacent_html(&mut self, position: &str, _html: &str) -> Result<()> {
        match position {
            "beforebegin" => {}
            "afterbegin" => {}
            "beforeend" => {}
            "afterend" => {}
            _ => {
                return Err(ElementError::InvalidOperation(format!(
                    "Invalid position: {}",
                    position
                )))
            }
        }
        Ok(())
    }

    pub fn insert_adjacent_text(&mut self, position: &str, _text: &str) -> Result<()> {
        match position {
            "beforebegin" => {}
            "afterbegin" => {}
            "beforeend" => {}
            "afterend" => {}
            _ => {
                return Err(ElementError::InvalidOperation(format!(
                    "Invalid position: {}",
                    position
                )))
            }
        }
        Ok(())
    }

    pub fn matches(&self, _selector: &str) -> Result<bool> {
        Ok(false)
    }

    pub fn closest(&self, _selector: &str) -> Result<Option<NodeId>> {
        Ok(None)
    }

    pub fn animate(
        &self,
        _keyframes: Vec<HashMap<String, String>>,
        options: AnimationOptions,
    ) -> AnimationId {
        let mut animations = self.animation_properties.write();
        let animation = Animation {
            name: format!("animation_{}", fastrand::u64(..)),
            duration: options.duration,
            timing_function: options.easing.clone(),
            delay: options.delay,
            iteration_count: options.iterations,
            direction: "normal".to_string(),
            fill_mode: options.fill.clone(),
            play_state: "running".to_string(),
        };
        animations.animations.push(animation.clone());
        AnimationId(animation.name)
    }

    pub fn get_animations(&self) -> Vec<Animation> {
        self.animation_properties.read().animations.clone()
    }

    pub fn get_computed_style_property(&self, property: &str) -> Option<String> {
        let computed_style = self.node.get_computed_style();
        let style = computed_style.read();
        style.get_property(property).cloned()
    }

    pub fn invalidate_style(&mut self) {
        self.node.invalidate_style();
    }

    pub fn invalidate_layout(&mut self) {
        self.node.invalidate_layout();
    }

    pub fn get_element_internals(&self) -> Arc<RwLock<ElementInternals>> {
        self.internals.clone()
    }

    pub fn attach_shadow(&mut self) -> Result<NodeId> {
        let shadow_root_id = NodeId::new();
        self.node.set_shadow_root(Some(shadow_root_id));
        Ok(shadow_root_id)
    }

    pub fn get_shadow_root(&self) -> Option<NodeId> {
        self.node.get_shadow_root()
    }

    pub fn request_fullscreen(&self) -> Result<()> {
        Ok(())
    }

    pub fn exit_fullscreen(&self) -> Result<()> {
        Ok(())
    }

    pub fn request_pointer_lock(&self) -> Result<()> {
        Ok(())
    }

    pub fn exit_pointer_lock(&self) -> Result<()> {
        Ok(())
    }

    pub fn get_memory_usage(&self) -> usize {
        let mut size = self.node.get_memory_usage();
        size += std::mem::size_of::<ElementProperties>();
        size += self.dataset.read().capacity() * std::mem::size_of::<(String, String)>();
        size += self.class_list.read().capacity() * std::mem::size_of::<String>();
        for (key, value) in self.dataset.read().iter() {
            size += key.capacity() + value.capacity();
        }
        for class in self.class_list.read().iter() {
            size += class.capacity();
        }
        size
    }
}

#[derive(Debug, Clone)]
pub struct AnimationOptions {
    pub duration: f64,
    pub delay: f64,
    pub iterations: f64,
    pub easing: String,
    pub fill: String,
}

impl Default for AnimationOptions {
    fn default() -> Self {
        Self {
            duration: 0.0,
            delay: 0.0,
            iterations: 1.0,
            easing: "linear".to_string(),
            fill: "none".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnimationId(pub String);

#[derive(Debug, Clone)]
pub struct ShadowRootInit {
    pub mode: ShadowRootMode,
    pub delegated_focus: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowRootMode {
    Open,
    Closed,
}

impl Default for ShadowRootInit {
    fn default() -> Self {
        Self {
            mode: ShadowRootMode::Open,
            delegated_focus: false,
        }
    }
}
