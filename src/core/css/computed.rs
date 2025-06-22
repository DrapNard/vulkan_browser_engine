use std::sync::Arc;
use std::collections::HashMap;
use parking_lot::RwLock;
use dashmap::DashMap;
use thiserror::Error;

use super::{CSSValue, ComputedValue, Color, LayoutContext};
use super::parser::{CSSRule, CSSStyleRule, CSSMediaRule};
use super::selector::SelectorEngine;
use crate::core::css::computed;
use crate::core::dom::{Document, NodeId};

#[derive(Error, Debug)]
pub enum ComputedStyleError {
    #[error("Property computation failed: {0}")]
    PropertyComputation(String),
    #[error("Value resolution failed: {0}")]
    ValueResolution(String),
    #[error("Cascade error: {0}")]
    Cascade(String),
    #[error("Inheritance error: {0}")]
    Inheritance(String),
}

pub type Result<T> = std::result::Result<T, ComputedStyleError>;

#[derive(Debug, Clone)]
pub struct PropertyDefinition {
    pub name: String,
    pub inherited: bool,
    pub initial_value: ComputedValue,
    pub applies_to: Vec<String>,
    pub percentages: PercentageBase,
    pub computed_value: ComputedValueType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PercentageBase {
    None,
    Width,
    Height,
    FontSize,
    LineHeight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputedValueType {
    AsSpecified,
    AbsoluteLength,
    NormalizedAngle,
    PercentageOrAbsoluteLength,
    ColorValue,
    ListOfComponentValues,
}

#[derive(Debug)]
pub struct ComputedStyles {
    properties: DashMap<String, ComputedValue>,
    specificity_map: DashMap<String, u32>,
    source_map: DashMap<String, String>,
    parent_styles: Option<Arc<ComputedStyles>>,
    context: LayoutContext,
    is_dirty: RwLock<bool>,
}

impl ComputedStyles {
    pub fn new(context: LayoutContext) -> Self {
        let mut styles = Self {
            properties: DashMap::new(),
            specificity_map: DashMap::new(),
            source_map: DashMap::new(),
            parent_styles: None,
            context,
            is_dirty: RwLock::new(true),
        };
        styles.initialize_defaults();
        styles
    }

    pub fn with_parent(context: LayoutContext, parent: Arc<ComputedStyles>) -> Self {
        let mut styles = Self {
            properties: DashMap::new(),
            specificity_map: DashMap::new(),
            source_map: DashMap::new(),
            parent_styles: Some(parent),
            context,
            is_dirty: RwLock::new(true),
        };
        styles.initialize_defaults();
        styles.inherit_from_parent();
        styles
    }

    fn initialize_defaults(&mut self) {
        for prop in Self::get_default_properties() {
            self.properties.insert(prop.name.clone(), prop.initial_value);
        }
    }

    fn inherit_from_parent(&self) {
        if let Some(ref p) = self.parent_styles {
            for name in Self::get_inherited_properties() {
                if let Some(v) = p.properties.get(&name) {
                    self.properties.insert(name.clone(), v.clone());
                }
            }
        }
    }

    pub fn set_property(&self, name: &str, value: ComputedValue, specificity: u32, source: &str) {
        if let Some(current) = self.specificity_map.get(name) {
            if specificity < *current {
                return;
            }
        }
        self.properties.insert(name.to_string(), value);
        self.specificity_map.insert(name.to_string(), specificity);
        self.source_map.insert(name.to_string(), source.to_string());
        *self.is_dirty.write() = true;
    }

    pub fn get_property(&self, name: &str) -> Option<ComputedValue> {
        self.properties.get(name).map(|e| e.clone())
    }

    pub fn get_computed_value(&self, name: &str) -> Result<ComputedValue> {
        if let Some(v) = self.get_property(name) {
            self.resolve_computed_value(name, &v)
        } else {
            Err(ComputedStyleError::PropertyComputation(format!("Property not found: {}", name)))
        }
    }

    fn resolve_computed_value(&self, property_name: &str, value: &ComputedValue) -> Result<ComputedValue> {
        match value {
            ComputedValue::Length(l) => Ok(ComputedValue::Length(*l)),
            ComputedValue::Percentage(p) => {
                let base = self.get_percentage_base(property_name);
                let resolved = self.resolve_percentage(*p, base)?;
                Ok(ComputedValue::Length(resolved))
            }
            ComputedValue::Auto => self.resolve_auto_value(property_name),
            ComputedValue::Initial => self.get_initial_value(property_name),
            ComputedValue::Inherit => self.get_inherited_value(property_name),
            ComputedValue::Unset => {
                if Self::is_inherited_property(property_name) {
                    self.get_inherited_value(property_name)
                } else {
                    self.get_initial_value(property_name)
                }
            }
            ComputedValue::Revert => self.get_user_agent_value(property_name),
            ComputedValue::Function { name, args } => self.resolve_function(name, args),
            ComputedValue::List(vals) => {
                let mut out = Vec::new();
                for v in vals {
                    out.push(self.resolve_computed_value(property_name, v)?);
                }
                Ok(ComputedValue::List(out))
            }
            _ => Ok(value.clone()),
        }
    }

    fn get_percentage_base(&self, property_name: &str) -> PercentageBase {
        match property_name {
            "width" | "min-width" | "max-width" | "left" | "right"
            | "margin-left" | "margin-right" | "padding-left" | "padding-right"
            | "border-left-width" | "border-right-width" => PercentageBase::Width,
            "height" | "min-height" | "max-height" | "top" | "bottom"
            | "margin-top" | "margin-bottom" | "padding-top" | "padding-bottom"
            | "border-top-width" | "border-bottom-width" => PercentageBase::Height,
            "font-size" => PercentageBase::FontSize,
            "line-height" => PercentageBase::LineHeight,
            _ => PercentageBase::None,
        }
    }

    fn resolve_percentage(&self, percentage: f32, base: PercentageBase) -> Result<f32> {
        let base_value = match base {
            PercentageBase::Width => self.context.containing_block_width,
            PercentageBase::Height => self.context.containing_block_height,
            PercentageBase::FontSize => self.context.font_size,
            PercentageBase::LineHeight => self.context.font_size * 1.2,
            PercentageBase::None => {
                return Err(ComputedStyleError::ValueResolution("Cannot resolve percentage without base".into()));
            }
        };
        Ok(base_value * percentage / 100.0)
    }

    fn resolve_auto_value(&self, property_name: &str) -> Result<ComputedValue> {
        match property_name {
            "width" => Ok(ComputedValue::Length(self.context.containing_block_width)),
            "height" => Ok(ComputedValue::Auto),
            "margin-left" | "margin-right" | "margin-top" | "margin-bottom" => Ok(ComputedValue::Length(0.0)),
            _ => Ok(ComputedValue::Auto),
        }
    }

    fn get_initial_value(&self, property_name: &str) -> Result<ComputedValue> {
        let def = Self::get_property_definition(property_name)
            .ok_or_else(|| ComputedStyleError::PropertyComputation(format!("Unknown property: {}", property_name)))?;
        Ok(def.initial_value.clone())
    }

    fn get_inherited_value(&self, property_name: &str) -> Result<ComputedValue> {
        if let Some(ref parent) = self.parent_styles {
            parent.get_computed_value(property_name)
        } else {
            self.get_initial_value(property_name)
        }
    }

    fn get_user_agent_value(&self, property_name: &str) -> Result<ComputedValue> {
        match property_name {
            "display" => Ok(ComputedValue::Keyword("block".into())),
            "color" => Ok(ComputedValue::Color(Color::BLACK)),
            "background-color" => Ok(ComputedValue::Color(Color::TRANSPARENT)),
            "font-family" => Ok(ComputedValue::String("serif".into())),
            "font-size" => Ok(ComputedValue::Length(16.0)),
            "font-weight" => Ok(ComputedValue::Number(400.0)),
            "line-height" => Ok(ComputedValue::Number(1.2)),
            _ => self.get_initial_value(property_name),
        }
    }

    fn resolve_function(&self, name: &str, args: &[ComputedValue]) -> Result<ComputedValue> {
        match name {
            "calc" => self.resolve_calc_function(args),
            "min" => self.resolve_min_function(args),
            "max" => self.resolve_max_function(args),
            "clamp" => self.resolve_clamp_function(args),
            "var" => self.resolve_var_function(args),
            "rgb" => self.resolve_rgb_function(args),
            "rgba" => self.resolve_rgba_function(args),
            "hsl" => self.resolve_hsl_function(args),
            "hsla" => self.resolve_hsla_function(args),
            _ => Ok(ComputedValue::Function { name: name.to_string(), args: args.to_vec() }),
        }
    }

    fn resolve_calc_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 1 {
            return Err(ComputedStyleError::ValueResolution("calc() requires one argument".into()));
        }
        if let ComputedValue::String(expr) = &args[0] {
            self.evaluate_calc_expression(expr)
        } else {
            Err(ComputedStyleError::ValueResolution("Invalid calc() argument".into()))
        }
    }

    fn evaluate_calc_expression(&self, expression: &str) -> Result<ComputedValue> {
        if let Some(pos) = expression.find('+') {
            let (l, r) = expression.split_at(pos);
            let left = self.parse_calc_value(l.trim())?;
            let right = self.parse_calc_value(r[1..].trim())?;
            return self.add_calc_values(&left, &right);
        }
        if let Some(pos) = expression.find('-') {
            let (l, r) = expression.split_at(pos);
            let left = self.parse_calc_value(l.trim())?;
            let right = self.parse_calc_value(r[1..].trim())?;
            return self.subtract_calc_values(&left, &right);
        }
        self.parse_calc_value(expression.trim())
    }

    fn parse_calc_value(&self, value: &str) -> Result<ComputedValue> {
        if value.ends_with("px") {
            let num: f32 = value[..value.len() - 2]
                .parse()
                .map_err(|_| ComputedStyleError::ValueResolution("Invalid number in calc()".into()))?;
            Ok(ComputedValue::Length(num))
        } else if value.ends_with('%') {
            let num: f32 = value[..value.len() - 1]
                .parse()
                .map_err(|_| ComputedStyleError::ValueResolution("Invalid percentage in calc()".into()))?;
            Ok(ComputedValue::Percentage(num))
        } else if value.ends_with("em") {
            let num: f32 = value[..value.len() - 2]
                .parse()
                .map_err(|_| ComputedStyleError::ValueResolution("Invalid em in calc()".into()))?;
            Ok(ComputedValue::Length(num * self.context.font_size))
        } else {
            let num: f32 = value
                .parse()
                .map_err(|_| ComputedStyleError::ValueResolution("Invalid number in calc()".into()))?;
            Ok(ComputedValue::Number(num))
        }
    }

    fn add_calc_values(&self, left: &ComputedValue, right: &ComputedValue) -> Result<ComputedValue> {
        match (left, right) {
            (ComputedValue::Length(a), ComputedValue::Length(b)) => Ok(ComputedValue::Length(a + b)),
            (ComputedValue::Number(a), ComputedValue::Number(b)) => Ok(ComputedValue::Number(a + b)),
            (ComputedValue::Percentage(a), ComputedValue::Percentage(b)) => Ok(ComputedValue::Percentage(a + b)),
            _ => Err(ComputedStyleError::ValueResolution("Incompatible types in calc()".into())),
        }
    }

    fn subtract_calc_values(&self, left: &ComputedValue, right: &ComputedValue) -> Result<ComputedValue> {
        match (left, right) {
            (ComputedValue::Length(a), ComputedValue::Length(b)) => Ok(ComputedValue::Length(a - b)),
            (ComputedValue::Number(a), ComputedValue::Number(b)) => Ok(ComputedValue::Number(a - b)),
            (ComputedValue::Percentage(a), ComputedValue::Percentage(b)) => Ok(ComputedValue::Percentage(a - b)),
            _ => Err(ComputedStyleError::ValueResolution("Incompatible types in calc()".into())),
        }
    }

    fn resolve_min_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.is_empty() {
            return Err(ComputedStyleError::ValueResolution("min() requires ≥1 args".into()));
        }
        let mut min_val: Option<f32> = None;
        for arg in args {
            let cv = self.resolve_computed_value("", arg)?;
            if let ComputedValue::Length(v) = cv {
                min_val = Some(min_val.map_or(v, |m| m.min(v)));
            } else {
                return Err(ComputedStyleError::ValueResolution("min() args must be lengths".into()));
            }
        }
        Ok(ComputedValue::Length(min_val.unwrap()))
    }

    fn resolve_max_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.is_empty() {
            return Err(ComputedStyleError::ValueResolution("max() requires ≥1 args".into()));
        }
        let mut max_val: Option<f32> = None;
        for arg in args {
            let cv = self.resolve_computed_value("", arg)?;
            if let ComputedValue::Length(v) = cv {
                max_val = Some(max_val.map_or(v, |m| m.max(v)));
            } else {
                return Err(ComputedStyleError::ValueResolution("max() args must be lengths".into()));
            }
        }
        Ok(ComputedValue::Length(max_val.unwrap()))
    }

    fn resolve_clamp_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 3 {
            return Err(ComputedStyleError::ValueResolution("clamp() requires 3 args".into()));
        }
        let min = self.resolve_computed_value("", &args[0])?;
        let val = self.resolve_computed_value("", &args[1])?;
        let max = self.resolve_computed_value("", &args[2])?;
        match (min, val, max) {
            (ComputedValue::Length(a), ComputedValue::Length(b), ComputedValue::Length(c)) => {
                Ok(ComputedValue::Length(b.max(a).min(c)))
            }
            _ => Err(ComputedStyleError::ValueResolution("clamp() args must be lengths".into())),
        }
    }

    fn resolve_var_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.is_empty() || args.len() > 2 {
            return Err(ComputedStyleError::ValueResolution("var() requires 1–2 args".into()));
        }
        if let ComputedValue::String(name) = &args[0] {
            if let Some(v) = self.properties.get(&format!("--{}", name)) {
                return Ok(v.clone());
            }
            if args.len() == 2 {
                return Ok(args[1].clone());
            }
        }
        Err(ComputedStyleError::ValueResolution("var() resolution failed".into()))
    }

    fn resolve_rgb_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 3 {
            return Err(ComputedStyleError::ValueResolution("rgb() requires 3 args".into()));
        }
        let r = self.resolve_color_component(&args[0])?;
        let g = self.resolve_color_component(&args[1])?;
        let b = self.resolve_color_component(&args[2])?;
        Ok(ComputedValue::Color(Color::from_rgb(r, g, b)))
    }

    fn resolve_rgba_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 4 {
            return Err(ComputedStyleError::ValueResolution("rgba() requires 4 args".into()));
        }
        let r = self.resolve_color_component(&args[0])?;
        let g = self.resolve_color_component(&args[1])?;
        let b = self.resolve_color_component(&args[2])?;
        let a = self.resolve_alpha_component(&args[3])?;
        Ok(ComputedValue::Color(Color::from_rgba(r, g, b, a)))
    }

    fn resolve_hsl_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 3 {
            return Err(ComputedStyleError::ValueResolution("hsl() requires 3 args".into()));
        }
        let h = self.resolve_hue_component(&args[0])?;
        let s = self.resolve_saturation_component(&args[1])?;
        let l = self.resolve_lightness_component(&args[2])?;
        Ok(ComputedValue::Color(Color::from_hsl(h, s, l)))
    }

    fn resolve_hsla_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 4 {
            return Err(ComputedStyleError::ValueResolution("hsla() requires 4 args".into()));
        }
        let mut c = match self.resolve_hsl_function(&args[..3])? {
            ComputedValue::Color(col) => col,
            _ => return Err(ComputedStyleError::ValueResolution("hsla() failed".into())),
        };
        c.a = self.resolve_alpha_component(&args[3])?;
        Ok(ComputedValue::Color(c))
    }

    fn resolve_color_component(&self, v: &ComputedValue) -> Result<u8> {
        match v {
            ComputedValue::Number(n) => Ok((*n as u8).min(255)),
            ComputedValue::Percentage(p) => Ok((p * 255.0 / 100.0) as u8),
            _ => Err(ComputedStyleError::ValueResolution("Invalid color component".into())),
        }
    }

    fn resolve_alpha_component(&self, v: &ComputedValue) -> Result<f32> {
        match v {
            ComputedValue::Number(n) => Ok(n.clamp(0.0, 1.0)),
            ComputedValue::Percentage(p) => Ok((p / 100.0).clamp(0.0, 1.0)),
            _ => Err(ComputedStyleError::ValueResolution("Invalid alpha component".into())),
        }
    }

    fn resolve_hue_component(&self, v: &ComputedValue) -> Result<f32> {
        match v {
            ComputedValue::Number(n) => Ok(n.rem_euclid(360.0)),
            _ => Err(ComputedStyleError::ValueResolution("Invalid hue component".into())),
        }
    }

    fn resolve_saturation_component(&self, v: &ComputedValue) -> Result<f32> {
        match v {
            ComputedValue::Percentage(p) => Ok((p / 100.0).clamp(0.0, 1.0)),
            _ => Err(ComputedStyleError::ValueResolution("Invalid saturation component".into())),
        }
    }

    fn resolve_lightness_component(&self, v: &ComputedValue) -> Result<f32> {
        match v {
            ComputedValue::Percentage(p) => Ok((p / 100.0).clamp(0.0, 1.0)),
            _ => Err(ComputedStyleError::ValueResolution("Invalid lightness component".into())),
        }
    }

    pub fn is_dirty(&self) -> bool {
        *self.is_dirty.read()
    }

    pub fn mark_clean(&self) {
        *self.is_dirty.write() = false;
    }

    pub fn mark_dirty(&self) {
        *self.is_dirty.write() = true;
    }

    pub fn get_custom_properties(&self) -> Vec<String> {
        let mut props = Vec::new();
        for r in self.properties.iter() {
            props.push(r.key().clone());
        }
        props
    }

    pub fn get_all_properties(&self) -> HashMap<String, ComputedValue> {
        let mut map = HashMap::new();
        for r in self.properties.iter() {
            map.insert(r.key().clone(), r.value().clone());
        }
        map
    }

    fn is_inherited_property(name: &str) -> bool {
        for inherit in Self::get_inherited_properties() {
            if inherit == name {
                return true;
            }
        }
        false
    }

    fn get_inherited_properties() -> Vec<String> {
        let mut v = Vec::new();
        v.push("color".to_string());
        v.push("font-family".to_string());
        v.push("font-size".to_string());
        v.push("font-style".to_string());
        v.push("font-variant".to_string());
        v.push("font-weight".to_string());
        v.push("font-stretch".to_string());
        v.push("font-size-adjust".to_string());
        v.push("font".to_string());
        v.push("line-height".to_string());
        v.push("text-align".to_string());
        v.push("text-indent".to_string());
        v.push("text-transform".to_string());
        v.push("white-space".to_string());
        v.push("word-spacing".to_string());
        v.push("letter-spacing".to_string());
        v.push("text-decoration".to_string());
        v.push("text-shadow".to_string());
        v.push("direction".to_string());
        v.push("writing-mode".to_string());
        v.push("list-style".to_string());
        v.push("list-style-image".to_string());
        v.push("list-style-position".to_string());
        v.push("list-style-type".to_string());
        v.push("quotes".to_string());
        v.push("cursor".to_string());
        v.push("visibility".to_string());
        v
    }

    fn get_default_properties() -> Vec<PropertyDefinition> {
        vec![
            PropertyDefinition {
                name: "display".into(),
                inherited: false,
                initial_value: ComputedValue::Keyword("block".into()),
                applies_to: vec!["all".into()],
                percentages: PercentageBase::None,
                computed_value: ComputedValueType::AsSpecified,
            },
            PropertyDefinition {
                name: "position".into(),
                inherited: false,
                initial_value: ComputedValue::Keyword("static".into()),
                applies_to: vec!["all".into()],
                percentages: PercentageBase::None,
                computed_value: ComputedValueType::AsSpecified,
            },
            PropertyDefinition {
                name: "width".into(),
                inherited: false,
                initial_value: ComputedValue::Auto,
                applies_to: vec!["all".into()],
                percentages: PercentageBase::Width,
                computed_value: ComputedValueType::PercentageOrAbsoluteLength,
            },
            PropertyDefinition {
                name: "height".into(),
                inherited: false,
                initial_value: ComputedValue::Auto,
                applies_to: vec!["all".into()],
                percentages: PercentageBase::Height,
                computed_value: ComputedValueType::PercentageOrAbsoluteLength,
            },
            PropertyDefinition {
                name: "margin".into(),
                inherited: false,
                initial_value: ComputedValue::Length(0.0),
                applies_to: vec!["all".into()],
                percentages: PercentageBase::Width,
                computed_value: ComputedValueType::PercentageOrAbsoluteLength,
            },
            PropertyDefinition {
                name: "padding".into(),
                inherited: false,
                initial_value: ComputedValue::Length(0.0),
                applies_to: vec!["all".into()],
                percentages: PercentageBase::Width,
                computed_value: ComputedValueType::PercentageOrAbsoluteLength,
            },
            PropertyDefinition {
                name: "color".into(),
                inherited: true,
                initial_value: ComputedValue::Color(Color::BLACK),
                applies_to: vec!["all".into()],
                percentages: PercentageBase::None,
                computed_value: ComputedValueType::ColorValue,
            },
            PropertyDefinition {
                name: "background-color".into(),
                inherited: false,
                initial_value: ComputedValue::Color(Color::TRANSPARENT),
                applies_to: vec!["all".into()],
                percentages: PercentageBase::None,
                computed_value: ComputedValueType::ColorValue,
            },
            PropertyDefinition {
                name: "font-size".into(),
                inherited: true,
                initial_value: ComputedValue::Length(16.0),
                applies_to: vec!["all".into()],
                percentages: PercentageBase::FontSize,
                computed_value: ComputedValueType::AbsoluteLength,
            },
            PropertyDefinition {
                name: "font-family".into(),
                inherited: true,
                initial_value: ComputedValue::String("serif".into()),
                applies_to: vec!["all".into()],
                percentages: PercentageBase::None,
                computed_value: ComputedValueType::AsSpecified,
            },
        ]
    }

    fn get_property_definition(name: &str) -> Option<PropertyDefinition> {
        for def in Self::get_default_properties() {
            if def.name == name {
                return Some(def);
            }
        }
        None
    }
}

pub struct StyleEngine {
    selector_engine: Arc<SelectorEngine>,
    style_cache: DashMap<NodeId, Arc<ComputedStyles>>,
    stylesheet_cache: RwLock<Vec<Arc<CSSRule>>>,
    media_queries: RwLock<Vec<CSSMediaRule>>,
    context_stack: RwLock<Vec<LayoutContext>>,
}

impl StyleEngine {
    pub fn new() -> Self {
        Self {
            selector_engine: Arc::new(SelectorEngine::new()),
            style_cache: DashMap::new(),
            stylesheet_cache: RwLock::new(Vec::new()),
            media_queries: RwLock::new(Vec::new()),
            context_stack: RwLock::new(vec![LayoutContext::default()]),
        }
    }

    pub fn compute_styles(&self, document: &Document) -> Result<()> {
        self.style_cache.clear();
        if let Some(root) = document.get_root_node() {
            let ctx = self.get_current_context();
            self.compute_styles_recursive(root, None, document, ctx)?;
        }
        Ok(())
    }

    fn compute_styles_recursive(
        &self,
        node: NodeId,
        parent: Option<Arc<ComputedStyles>>,
        document: &Document,
        context: LayoutContext,
    ) -> Result<()> {
        let computed = if let Some(p) = parent {
            Arc::new(ComputedStyles::with_parent(context.clone(), p.clone()))
        } else {
            Arc::new(ComputedStyles::new(context.clone()))
        };
        self.apply_matching_rules(node, &computed, document)?;
        self.style_cache.insert(node, computed.clone());
        for &child in &document.get_children(node) {
            self.compute_styles_recursive(child, Some(computed.clone()), document, context.clone())?;
        }
        Ok(())
    }

    fn apply_matching_rules(
        &self,
        node: NodeId,
        computed: &ComputedStyles,
        document: &Document,
    ) -> Result<()> {
        let sheets = self.stylesheet_cache.read();
        for rule in sheets.iter() {
            match rule.as_ref() {
                CSSRule::Style(style_rule) => {
                    for selector in &style_rule.selectors {
                        if self.selector_engine
                            .matches(&format!("{:?}", selector), node, document)
                            .map_err(|e| ComputedStyleError::Cascade(e.to_string()))?
                        {
                            self.apply_style_rule(style_rule, computed)?;
                            break;
                        }
                    }
                }
                CSSRule::Media(media_rule) => {
                    if self.evaluate_media_query(&media_rule.media_query) {
                        for nested in &media_rule.rules {
                            if let CSSRule::Style(style_rule) = nested {
                                for selector in &style_rule.selectors {
                                    if self.selector_engine
                                        .matches(&format!("{:?}", selector), node, document)
                                        .map_err(|e| ComputedStyleError::Cascade(e.to_string()))?
                                    {
                                        self.apply_style_rule(style_rule, computed)?;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn apply_style_rule(
        &self,
        style_rule: &CSSStyleRule,
        computed: &ComputedStyles,
    ) -> Result<()> {
        for (prop, css_value , _important) in &style_rule.declarations.properties {
            let cv = self.convert_css_value_to_computed(css_value)?;
            computed.set_property(prop, cv, style_rule.specificity, "author");
        }
        Ok(())
    }

    fn convert_css_value_to_computed(&self, css_value: &CSSValue) -> Result<ComputedValue> {
        Ok(css_value.computed.clone())
    }

    fn evaluate_media_query(&self, _mq: &crate::core::css::parser::MediaQuery) -> bool {
        true
    }

    pub fn get_computed_styles(&self, node: NodeId) -> Option<Arc<ComputedStyles>> {
        self.style_cache.get(&node).map(|e| e.clone())
    }

    pub fn add_stylesheet(&self, rules: Vec<CSSRule>) {
        let mut ss = self.stylesheet_cache.write();
        for rule in rules {
            ss.push(Arc::new(rule));
        }
    }

    pub fn invalidate_node(&self, node: NodeId) {
        self.style_cache.remove(&node);
        self.selector_engine.invalidate_node_cache(node);
    }

    pub fn invalidate_all(&self) {
        self.style_cache.clear();
        self.selector_engine.invalidate_cache();
    }

    fn get_current_context(&self) -> LayoutContext {
        self.context_stack.read().last().cloned().unwrap_or_default()
    }

    pub fn push_context(&self, ctx: LayoutContext) {
        self.context_stack.write().push(ctx);
    }

    pub fn pop_context(&self) {
        let mut stk = self.context_stack.write();
        if stk.len() > 1 {
            stk.pop();
        }
    }

    pub fn get_cache_stats(&self) -> serde_json::Value {
        let sel_stats = self.selector_engine.get_cache_stats();
        serde_json::json!({
            "computed_styles_cache_size": self.style_cache.len(),
            "stylesheet_count": self.stylesheet_cache.read().len(),
            "media_queries_count": self.media_queries.read().len(),
            "context_stack_depth": self.context_stack.read().len(),
            "selector_engine": sel_stats,
        })
    }
}
