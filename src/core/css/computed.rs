use std::sync::Arc;
use std::collections::HashMap;
use parking_lot::RwLock;
use dashmap::DashMap;
use thiserror::Error;

use super::{CSSValue, ComputedValue, Color, LayoutContext};
use super::parser::{CSSRule, CSSStyleRule, CSSMediaRule};
use super::selector::SelectorEngine;
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
        for property in Self::get_default_properties() {
            self.properties.insert(property.name.clone(), property.initial_value);
        }
    }

    fn inherit_from_parent(&self) {
        if let Some(ref parent) = self.parent_styles {
            for property_name in Self::get_inherited_properties() {
                if let Some(parent_value) = parent.properties.get(&property_name) {
                    self.properties.insert(property_name, parent_value.clone());
                }
            }
        }
    }

    pub fn set_property(&self, name: &str, value: ComputedValue, specificity: u32, source: &str) {
        if let Some(current_specificity) = self.specificity_map.get(name) {
            if specificity < *current_specificity {
                return;
            }
        }

        self.properties.insert(name.to_string(), value);
        self.specificity_map.insert(name.to_string(), specificity);
        self.source_map.insert(name.to_string(), source.to_string());
        *self.is_dirty.write() = true;
    }

    pub fn get_property(&self, name: &str) -> Option<ComputedValue> {
        self.properties.get(name).map(|entry| entry.clone())
    }

    pub fn get_computed_value(&self, name: &str) -> Result<ComputedValue> {
        if let Some(value) = self.get_property(name) {
            self.resolve_computed_value(name, &value)
        } else {
            Err(ComputedStyleError::PropertyComputation(format!("Property not found: {}", name)))
        }
    }

    fn resolve_computed_value(&self, property_name: &str, value: &ComputedValue) -> Result<ComputedValue> {
        match value {
            ComputedValue::Length(length) => Ok(ComputedValue::Length(*length)),
            ComputedValue::Percentage(percentage) => {
                let base = self.get_percentage_base(property_name);
                let resolved = self.resolve_percentage(*percentage, base)?;
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
            ComputedValue::List(values) => {
                let resolved_values: Result<Vec<ComputedValue>> = values
                    .iter()
                    .map(|v| self.resolve_computed_value(property_name, v))
                    .collect();
                Ok(ComputedValue::List(resolved_values?))
            }
            _ => Ok(value.clone()),
        }
    }

    fn get_percentage_base(&self, property_name: &str) -> PercentageBase {
        match property_name {
            "width" | "min-width" | "max-width" | "left" | "right" | "margin-left" | "margin-right" | 
            "padding-left" | "padding-right" | "border-left-width" | "border-right-width" => PercentageBase::Width,
            
            "height" | "min-height" | "max-height" | "top" | "bottom" | "margin-top" | "margin-bottom" |
            "padding-top" | "padding-bottom" | "border-top-width" | "border-bottom-width" => PercentageBase::Height,
            
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
                return Err(ComputedStyleError::ValueResolution(
                    "Cannot resolve percentage without base".to_string()
                ));
            }
        };

        Ok(base_value * percentage / 100.0)
    }

    fn resolve_auto_value(&self, property_name: &str) -> Result<ComputedValue> {
        match property_name {
            "width" => Ok(ComputedValue::Length(self.context.containing_block_width)),
            "height" => Ok(ComputedValue::Auto),
            "margin-left" | "margin-right" | "margin-top" | "margin-bottom" => {
                Ok(ComputedValue::Length(0.0))
            }
            _ => Ok(ComputedValue::Auto),
        }
    }

    fn get_initial_value(&self, property_name: &str) -> Result<ComputedValue> {
        let property = Self::get_property_definition(property_name)
            .ok_or_else(|| ComputedStyleError::PropertyComputation(
                format!("Unknown property: {}", property_name)
            ))?;
        
        Ok(property.initial_value.clone())
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
            "display" => Ok(ComputedValue::Keyword("block".to_string())),
            "color" => Ok(ComputedValue::Color(Color::BLACK)),
            "background-color" => Ok(ComputedValue::Color(Color::TRANSPARENT)),
            "font-family" => Ok(ComputedValue::String("serif".to_string())),
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
            "linear-gradient" | "radial-gradient" | "url" => {
                Ok(ComputedValue::Function { name: name.to_string(), args: args.to_vec() })
            }
            _ => Ok(ComputedValue::Function { name: name.to_string(), args: args.to_vec() }),
        }
    }

    fn resolve_calc_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 1 {
            return Err(ComputedStyleError::ValueResolution("calc() requires one argument".to_string()));
        }

        match &args[0] {
            ComputedValue::String(expression) => self.evaluate_calc_expression(expression),
            _ => Err(ComputedStyleError::ValueResolution("Invalid calc() argument".to_string())),
        }
    }

    fn evaluate_calc_expression(&self, expression: &str) -> Result<ComputedValue> {
        if expression.contains('+') {
            let parts: Vec<&str> = expression.split('+').map(|s| s.trim()).collect();
            if parts.len() == 2 {
                let left = self.parse_calc_value(parts[0])?;
                let right = self.parse_calc_value(parts[1])?;
                return self.add_calc_values(&left, &right);
            }
        }

        if expression.contains('-') {
            let parts: Vec<&str> = expression.split('-').map(|s| s.trim()).collect();
            if parts.len() == 2 {
                let left = self.parse_calc_value(parts[0])?;
                let right = self.parse_calc_value(parts[1])?;
                return self.subtract_calc_values(&left, &right);
            }
        }

        self.parse_calc_value(expression)
    }

    fn parse_calc_value(&self, value: &str) -> Result<ComputedValue> {
        let value = value.trim();
        
        if value.ends_with("px") {
            let num: f32 = value.trim_end_matches("px").parse()
                .map_err(|_| ComputedStyleError::ValueResolution("Invalid number in calc()".to_string()))?;
            Ok(ComputedValue::Length(num))
        } else if value.ends_with("%") {
            let num: f32 = value.trim_end_matches("%").parse()
                .map_err(|_| ComputedStyleError::ValueResolution("Invalid percentage in calc()".to_string()))?;
            Ok(ComputedValue::Percentage(num))
        } else if value.ends_with("em") {
            let num: f32 = value.trim_end_matches("em").parse()
                .map_err(|_| ComputedStyleError::ValueResolution("Invalid em value in calc()".to_string()))?;
            Ok(ComputedValue::Length(num * self.context.font_size))
        } else {
            let num: f32 = value.parse()
                .map_err(|_| ComputedStyleError::ValueResolution("Invalid number in calc()".to_string()))?;
            Ok(ComputedValue::Number(num))
        }
    }

    fn add_calc_values(&self, left: &ComputedValue, right: &ComputedValue) -> Result<ComputedValue> {
        match (left, right) {
            (ComputedValue::Length(a), ComputedValue::Length(b)) => Ok(ComputedValue::Length(a + b)),
            (ComputedValue::Number(a), ComputedValue::Number(b)) => Ok(ComputedValue::Number(a + b)),
            (ComputedValue::Percentage(a), ComputedValue::Percentage(b)) => Ok(ComputedValue::Percentage(a + b)),
            _ => Err(ComputedStyleError::ValueResolution("Cannot add incompatible types in calc()".to_string())),
        }
    }

    fn subtract_calc_values(&self, left: &ComputedValue, right: &ComputedValue) -> Result<ComputedValue> {
        match (left, right) {
            (ComputedValue::Length(a), ComputedValue::Length(b)) => Ok(ComputedValue::Length(a - b)),
            (ComputedValue::Number(a), ComputedValue::Number(b)) => Ok(ComputedValue::Number(a - b)),
            (ComputedValue::Percentage(a), ComputedValue::Percentage(b)) => Ok(ComputedValue::Percentage(a - b)),
            _ => Err(ComputedStyleError::ValueResolution("Cannot subtract incompatible types in calc()".to_string())),
        }
    }

    fn resolve_min_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.is_empty() {
            return Err(ComputedStyleError::ValueResolution("min() requires at least one argument".to_string()));
        }

        let mut min_value = None;
        
        for arg in args {
            let resolved = self.resolve_computed_value("", arg)?;
            if let ComputedValue::Length(value) = resolved {
                if min_value.is_none() || value < min_value.unwrap() {
                    min_value = Some(value);
                }
            } else {
                return Err(ComputedStyleError::ValueResolution("min() arguments must be lengths".to_string()));
            }
        }

        Ok(ComputedValue::Length(min_value.unwrap()))
    }

    fn resolve_max_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.is_empty() {
            return Err(ComputedStyleError::ValueResolution("max() requires at least one argument".to_string()));
        }

        let mut max_value = None;
        
        for arg in args {
            let resolved = self.resolve_computed_value("", arg)?;
            if let ComputedValue::Length(value) = resolved {
                if max_value.is_none() || value > max_value.unwrap() {
                    max_value = Some(value);
                }
            } else {
                return Err(ComputedStyleError::ValueResolution("max() arguments must be lengths".to_string()));
            }
        }

        Ok(ComputedValue::Length(max_value.unwrap()))
    }

    fn resolve_clamp_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 3 {
            return Err(ComputedStyleError::ValueResolution("clamp() requires exactly 3 arguments".to_string()));
        }

        let min = self.resolve_computed_value("", &args[0])?;
        let preferred = self.resolve_computed_value("", &args[1])?;
        let max = self.resolve_computed_value("", &args[2])?;

        match (min, preferred, max) {
            (ComputedValue::Length(min_val), ComputedValue::Length(pref_val), ComputedValue::Length(max_val)) => {
                let clamped = pref_val.min(max_val).max(min_val);
                Ok(ComputedValue::Length(clamped))
            }
            _ => Err(ComputedStyleError::ValueResolution("clamp() arguments must be lengths".to_string())),
        }
    }

    fn resolve_var_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.is_empty() || args.len() > 2 {
            return Err(ComputedStyleError::ValueResolution("var() requires 1 or 2 arguments".to_string()));
        }

        if let ComputedValue::String(custom_property_name) = &args[0] {
            if let Some(value) = self.get_custom_property(custom_property_name) {
                return Ok(value);
            }

            if args.len() == 2 {
                return Ok(args[1].clone());
            }
        }

        Err(ComputedStyleError::ValueResolution("Custom property not found".to_string()))
    }

    fn resolve_rgb_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 3 {
            return Err(ComputedStyleError::ValueResolution("rgb() requires 3 arguments".to_string()));
        }

        let r = self.resolve_color_component(&args[0])?;
        let g = self.resolve_color_component(&args[1])?;
        let b = self.resolve_color_component(&args[2])?;

        Ok(ComputedValue::Color(Color::from_rgb(r, g, b)))
    }

    fn resolve_rgba_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 4 {
            return Err(ComputedStyleError::ValueResolution("rgba() requires 4 arguments".to_string()));
        }

        let r = self.resolve_color_component(&args[0])?;
        let g = self.resolve_color_component(&args[1])?;
        let b = self.resolve_color_component(&args[2])?;
        let a = self.resolve_alpha_component(&args[3])?;

        Ok(ComputedValue::Color(Color::from_rgba(r, g, b, a)))
    }

    fn resolve_hsl_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 3 {
            return Err(ComputedStyleError::ValueResolution("hsl() requires 3 arguments".to_string()));
        }

        let h = self.resolve_hue_component(&args[0])?;
        let s = self.resolve_saturation_component(&args[1])?;
        let l = self.resolve_lightness_component(&args[2])?;

        Ok(ComputedValue::Color(Color::from_hsl(h, s, l)))
    }

    fn resolve_hsla_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 4 {
            return Err(ComputedStyleError::ValueResolution("hsla() requires 4 arguments".to_string()));
        }

        let h = self.resolve_hue_component(&args[0])?;
        let s = self.resolve_saturation_component(&args[1])?;
        let l = self.resolve_lightness_component(&args[2])?;
        let a = self.resolve_alpha_component(&args[3])?;

        let mut color = Color::from_hsl(h, s, l);
        color.a = a;

        Ok(ComputedValue::Color(color))
    }

    fn resolve_color_component(&self, value: &ComputedValue) -> Result<u8> {
        match value {
            ComputedValue::Number(n) => Ok((*n as u8).min(255)),
            ComputedValue::Percentage(p) => Ok((p * 255.0 / 100.0) as u8),
            _ => Err(ComputedStyleError::ValueResolution("Invalid color component".to_string())),
        }
    }

    fn resolve_alpha_component(&self, value: &ComputedValue) -> Result<f32> {
        match value {
            ComputedValue::Number(n) => Ok(n.clamp(0.0, 1.0)),
            ComputedValue::Percentage(p) => Ok((p / 100.0).clamp(0.0, 1.0)),
            _ => Err(ComputedStyleError::ValueResolution("Invalid alpha component".to_string())),
        }
    }

    fn resolve_hue_component(&self, value: &ComputedValue) -> Result<f32> {
        match value {
            ComputedValue::Number(n) => Ok(n % 360.0),
            _ => Err(ComputedStyleError::ValueResolution("Invalid hue component".to_string())),
        }
    }

    fn resolve_saturation_component(&self, value: &ComputedValue) -> Result<f32> {
        match value {
            ComputedValue::Percentage(p) => Ok((p / 100.0).clamp(0.0, 1.0)),
            _ => Err(ComputedStyleError::ValueResolution("Invalid saturation component".to_string())),
        }
    }

    fn resolve_lightness_component(&self, value: &ComputedValue) -> Result<f32> {
        match value {
            ComputedValue::Percentage(p) => Ok((p / 100.0).clamp(0.0, 1.0)),
            _ => Err(ComputedStyleError::ValueResolution("Invalid lightness component".to_string())),
        }
    }

    fn get_custom_property(&self, name: &str) -> Option<ComputedValue> {
        self.properties.get(&format!("--{}", name)).map(|entry| entry.clone())
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

    pub fn is_inherited_property(property_name: &str) -> bool {
        Self::get_inherited_properties().contains(&property_name.to_string())
    }

    fn get_inherited_properties() -> Vec<String> {
        vec![
            "color".to_string(),
            "font-family".to_string(),
            "font-size".to_string(),
            "font-style".to_string(),
            "font-variant".to_string(),
            "font-weight".to_string(),
            "font-stretch".to_string(),
            "font-size-adjust".to_string(),
            "font".to_string(),
            "line-height".to_string(),
            "text-align".to_string(),
            "text-indent".to_string(),
            "text-transform".to_string(),
            "white-space".to_string(),
            "word-spacing".to_string(),
            "letter-spacing".to_string(),
            "text-decoration".to_string(),
            "text-shadow".to_string(),
            "direction".to_string(),
            "writing-mode".to_string(),
            "list-style".to_string(),
            "list-style-image".to_string(),
            "list-style-position".to_string(),
            "list-style-type".to_string(),
            "quotes".to_string(),
            "cursor".to_string(),
            "visibility".to_string(),
        ]
    }

    fn get_default_properties() -> Vec<PropertyDefinition> {
        vec![
            PropertyDefinition {
                name: "display".to_string(),
                inherited: false,
                initial_value: ComputedValue::Keyword("block".to_string()),
                applies_to: vec!["all".to_string()],
                percentages: PercentageBase::None,
                computed_value: ComputedValueType::AsSpecified,
            },
            PropertyDefinition {
                name: "position".to_string(),
                inherited: false,
                initial_value: ComputedValue::Keyword("static".to_string()),
                applies_to: vec!["all".to_string()],
                percentages: PercentageBase::None,
                computed_value: ComputedValueType::AsSpecified,
            },
            PropertyDefinition {
                name: "width".to_string(),
                inherited: false,
                initial_value: ComputedValue::Auto,
                applies_to: vec!["all".to_string()],
                percentages: PercentageBase::Width,
                computed_value: ComputedValueType::PercentageOrAbsoluteLength,
            },
            PropertyDefinition {
                name: "height".to_string(),
                inherited: false,
                initial_value: ComputedValue::Auto,
                applies_to: vec!["all".to_string()],
                percentages: PercentageBase::Height,
                computed_value: ComputedValueType::PercentageOrAbsoluteLength,
            },
            PropertyDefinition {
                name: "margin".to_string(),
                inherited: false,
                initial_value: ComputedValue::Length(0.0),
                applies_to: vec!["all".to_string()],
                percentages: PercentageBase::Width,
                computed_value: ComputedValueType::PercentageOrAbsoluteLength,
            },
            PropertyDefinition {
                name: "padding".to_string(),
                inherited: false,
                initial_value: ComputedValue::Length(0.0),
                applies_to: vec!["all".to_string()],
                percentages: PercentageBase::Width,
                computed_value: ComputedValueType::PercentageOrAbsoluteLength,
            },
            PropertyDefinition {
                name: "color".to_string(),
                inherited: true,
                initial_value: ComputedValue::Color(Color::BLACK),
                applies_to: vec!["all".to_string()],
                percentages: PercentageBase::None,
                computed_value: ComputedValueType::ColorValue,
            },
            PropertyDefinition {
                name: "background-color".to_string(),
                inherited: false,
                initial_value: ComputedValue::Color(Color::TRANSPARENT),
                applies_to: vec!["all".to_string()],
                percentages: PercentageBase::None,
                computed_value: ComputedValueType::ColorValue,
            },
            PropertyDefinition {
                name: "font-size".to_string(),
                inherited: true,
                initial_value: ComputedValue::Length(16.0),
                applies_to: vec!["all".to_string()],
                percentages: PercentageBase::FontSize,
                computed_value: ComputedValueType::AbsoluteLength,
            },
            PropertyDefinition {
                name: "font-family".to_string(),
                inherited: true,
                initial_value: ComputedValue::String("serif".to_string()),
                applies_to: vec!["all".to_string()],
                percentages: PercentageBase::None,
                computed_value: ComputedValueType::AsSpecified,
            },
        ]
    }

    fn get_property_definition(name: &str) -> Option<PropertyDefinition> {
        Self::get_default_properties()
            .into_iter()
            .find(|prop| prop.name == name)
    }

    pub fn get_all_properties(&self) -> HashMap<String, ComputedValue> {
        self.properties.iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    pub fn get_used_value(&self, property_name: &str) -> Result<f32> {
        let computed = self.get_computed_value(property_name)?;
        
        match computed {
            ComputedValue::Length(value) => Ok(value),
            ComputedValue::Number(value) => Ok(value),
            ComputedValue::Percentage(percentage) => {
                let base = self.get_percentage_base(property_name);
                self.resolve_percentage(percentage, base)
            }
            ComputedValue::Auto => {
                match property_name {
                    "width" => Ok(self.context.containing_block_width),
                    "height" => Ok(0.0),
                    _ => Ok(0.0),
                }
            }
            _ => Err(ComputedStyleError::ValueResolution(
                format!("Cannot convert {} to used value", property_name)
            )),
        }
    }

    pub fn clone_with_new_context(&self, context: LayoutContext) -> ComputedStyles {
        ComputedStyles {
            properties: DashMap::new(),
            specificity_map: DashMap::new(),
            source_map: DashMap::new(),
            parent_styles: self.parent_styles.clone(),
            context,
            is_dirty: RwLock::new(true),
        }
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

    pub async fn compute_styles(&self, document: &Document) -> Result<()> {
        self.style_cache.clear();
        
        if let Some(root_id) = document.get_root_node() {
            let context = self.get_current_context();
            self.compute_styles_recursive(root_id, None, document, context).await?;
        }

        Ok(())
    }

    async fn compute_styles_recursive(
        &self,
        node_id: NodeId,
        parent_styles: Option<Arc<ComputedStyles>>,
        document: &Document,
        context: LayoutContext,
    ) -> Result<()> {
        let computed_styles = self.compute_styles_for_node(node_id, parent_styles.as_ref(), document)?;
        
        let children = document.get_children(node_id);
        for &child_id in &children {
            Box::pin(self.compute_styles_recursive(
                child_id, 
                Some(computed_styles.clone()), 
                document, 
                context.clone()
            )).await?;
        }
        
        Ok(())
    }

    fn compute_styles_for_node(
        &self,
        node_id: NodeId,
        parent_styles: Option<&Arc<ComputedStyles>>,
        document: &Document,
    ) -> Result<Arc<ComputedStyles>> {
        let context = self.get_current_context();
        
        let computed_styles = if let Some(parent) = parent_styles {
            Arc::new(ComputedStyles::with_parent(context, parent.clone()))
        } else {
            Arc::new(ComputedStyles::new(context))
        };

        futures::executor::block_on(self.apply_matching_rules(node_id, &computed_styles, document))?;
        
        self.style_cache.insert(node_id, computed_styles.clone());
        Ok(computed_styles)
    }

    async fn apply_matching_rules(
        &self,
        node_id: NodeId,
        computed_styles: &ComputedStyles,
        document: &Document,
    ) -> Result<()> {
        let stylesheets = self.stylesheet_cache.read();
        
        for rule in stylesheets.iter() {
            match rule.as_ref() {
                CSSRule::Style(style_rule) => {
                    for selector in &style_rule.selectors {
                        if self.selector_engine.matches(&format!("{:?}", selector), node_id, document)
                            .map_err(|e| ComputedStyleError::Cascade(e.to_string()))? {
                            
                            self.apply_style_rule(style_rule, computed_styles)?;
                            break;
                        }
                    }
                }
                CSSRule::Media(media_rule) => {
                    if self.evaluate_media_query(&media_rule.media_query) {
                        for nested_rule in &media_rule.rules {
                            if let CSSRule::Style(style_rule) = nested_rule {
                                for selector in &style_rule.selectors {
                                    if self.selector_engine.matches(&format!("{:?}", selector), node_id, document)
                                        .map_err(|e| ComputedStyleError::Cascade(e.to_string()))? {
                                        
                                        self.apply_style_rule(style_rule, computed_styles)?;
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
        computed_styles: &ComputedStyles,
    ) -> Result<()> {
        let properties = style_rule.declarations.get_all_properties();
        
        for (property_name, css_value) in properties {
            let computed_value = self.convert_css_value_to_computed(&css_value)?;
            computed_styles.set_property(
                &property_name,
                computed_value,
                style_rule.specificity,
                "author",
            );
        }

        Ok(())
    }

    fn convert_css_value_to_computed(&self, css_value: &CSSValue) -> Result<ComputedValue> {
        Ok(css_value.computed.clone())
    }

    fn evaluate_media_query(&self, _media_query: &crate::core::css::parser::MediaQuery) -> bool {
        true
    }

    pub fn get_computed_styles(&self, node_id: NodeId) -> Option<Arc<ComputedStyles>> {
        self.style_cache.get(&node_id).map(|entry| entry.clone())
    }

    pub fn add_stylesheet(&self, rules: Vec<CSSRule>) {
        let mut stylesheets = self.stylesheet_cache.write();
        for rule in rules {
            stylesheets.push(Arc::new(rule));
        }
    }

    pub fn invalidate_node(&self, node_id: NodeId) {
        self.style_cache.remove(&node_id);
        self.selector_engine.invalidate_node_cache(node_id);
    }

    pub fn invalidate_all(&self) {
        self.style_cache.clear();
        self.selector_engine.invalidate_cache();
    }

    fn get_current_context(&self) -> LayoutContext {
        self.context_stack.read().last().cloned().unwrap_or_default()
    }

    pub fn push_context(&self, context: LayoutContext) {
        self.context_stack.write().push(context);
    }

    pub fn pop_context(&self) {
        let mut stack = self.context_stack.write();
        if stack.len() > 1 {
            stack.pop();
        }
    }

    pub fn get_cache_stats(&self) -> serde_json::Value {
        let selector_stats = self.selector_engine.get_cache_stats();
        
        serde_json::json!({
            "computed_styles_cache_size": self.style_cache.len(),
            "stylesheet_count": self.stylesheet_cache.read().len(),
            "media_queries_count": self.media_queries.read().len(),
            "context_stack_depth": self.context_stack.read().len(),
            "selector_engine": selector_stats,
        })
    }
}