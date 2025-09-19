use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use thiserror::Error;

use super::parser::{CSSMediaRule, CSSRule, CSSStyleRule};
use super::selector::SelectorEngine;
use super::{CSSUnit, Color, ComputedValue, LayoutContext};
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
    pub name: &'static str,
    pub inherited: bool,
    pub initial_value: ComputedValue,
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

static INHERITED_PROPERTIES: &[&str] = &[
    "color",
    "cursor",
    "direction",
    "font",
    "font-family",
    "font-size",
    "font-size-adjust",
    "font-stretch",
    "font-style",
    "font-variant",
    "font-weight",
    "letter-spacing",
    "line-height",
    "list-style",
    "list-style-image",
    "list-style-position",
    "list-style-type",
    "quotes",
    "text-align",
    "text-decoration",
    "text-indent",
    "text-shadow",
    "text-transform",
    "visibility",
    "white-space",
    "word-spacing",
    "writing-mode",
];

static DEFAULT_PROPERTIES: LazyLock<Vec<PropertyDefinition>> = LazyLock::new(|| {
    vec![
        PropertyDefinition {
            name: "display",
            inherited: false,
            initial_value: ComputedValue::Keyword("block".to_string()),
            percentages: PercentageBase::None,
            computed_value: ComputedValueType::AsSpecified,
        },
        PropertyDefinition {
            name: "position",
            inherited: false,
            initial_value: ComputedValue::Keyword("static".to_string()),
            percentages: PercentageBase::None,
            computed_value: ComputedValueType::AsSpecified,
        },
        PropertyDefinition {
            name: "width",
            inherited: false,
            initial_value: ComputedValue::Auto,
            percentages: PercentageBase::Width,
            computed_value: ComputedValueType::PercentageOrAbsoluteLength,
        },
        PropertyDefinition {
            name: "height",
            inherited: false,
            initial_value: ComputedValue::Auto,
            percentages: PercentageBase::Height,
            computed_value: ComputedValueType::PercentageOrAbsoluteLength,
        },
        PropertyDefinition {
            name: "margin",
            inherited: false,
            initial_value: ComputedValue::Length(0.0),
            percentages: PercentageBase::Width,
            computed_value: ComputedValueType::PercentageOrAbsoluteLength,
        },
        PropertyDefinition {
            name: "padding",
            inherited: false,
            initial_value: ComputedValue::Length(0.0),
            percentages: PercentageBase::Width,
            computed_value: ComputedValueType::PercentageOrAbsoluteLength,
        },
        PropertyDefinition {
            name: "color",
            inherited: true,
            initial_value: ComputedValue::Color(Color::BLACK),
            percentages: PercentageBase::None,
            computed_value: ComputedValueType::ColorValue,
        },
        PropertyDefinition {
            name: "background-color",
            inherited: false,
            initial_value: ComputedValue::Color(Color::TRANSPARENT),
            percentages: PercentageBase::None,
            computed_value: ComputedValueType::ColorValue,
        },
        PropertyDefinition {
            name: "font-size",
            inherited: true,
            initial_value: ComputedValue::Length(16.0),
            percentages: PercentageBase::FontSize,
            computed_value: ComputedValueType::AbsoluteLength,
        },
        PropertyDefinition {
            name: "font-family",
            inherited: true,
            initial_value: ComputedValue::String("serif".to_string()),
            percentages: PercentageBase::None,
            computed_value: ComputedValueType::AsSpecified,
        },
    ]
});

static UNIT_STRINGS: LazyLock<HashMap<CSSUnit, &'static str>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    map.insert(CSSUnit::Px, "px");
    map.insert(CSSUnit::Em, "em");
    map.insert(CSSUnit::Rem, "rem");
    map.insert(CSSUnit::Vh, "vh");
    map.insert(CSSUnit::Vw, "vw");
    map.insert(CSSUnit::Vmin, "vmin");
    map.insert(CSSUnit::Vmax, "vmax");
    map.insert(CSSUnit::Percent, "%");
    map.insert(CSSUnit::Pt, "pt");
    map.insert(CSSUnit::Pc, "pc");
    map.insert(CSSUnit::In, "in");
    map.insert(CSSUnit::Cm, "cm");
    map.insert(CSSUnit::Mm, "mm");
    map.insert(CSSUnit::Ex, "ex");
    map.insert(CSSUnit::Ch, "ch");
    map.insert(CSSUnit::Q, "q");
    map.insert(CSSUnit::Deg, "deg");
    map.insert(CSSUnit::Rad, "rad");
    map.insert(CSSUnit::Grad, "grad");
    map.insert(CSSUnit::Turn, "turn");
    map.insert(CSSUnit::S, "s");
    map.insert(CSSUnit::Ms, "ms");
    map.insert(CSSUnit::Hz, "hz");
    map.insert(CSSUnit::Khz, "khz");
    map.insert(CSSUnit::Dpi, "dpi");
    map.insert(CSSUnit::Dpcm, "dpcm");
    map.insert(CSSUnit::Dppx, "dppx");
    map.insert(CSSUnit::Fr, "fr");
    map
});

trait CSSValueParser {
    fn parse_raw(&self, value: &str, important: bool) -> Result<ComputedValue>;
    fn extract_unit(&self, value: &str) -> Option<CSSUnit>;
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

impl CSSValueParser for ComputedStyles {
    fn parse_raw(&self, value: &str, _important: bool) -> Result<ComputedValue> {
        let trimmed = value.trim();

        if trimmed.is_empty() {
            return Ok(ComputedValue::None);
        }

        match trimmed {
            "auto" => return Ok(ComputedValue::Auto),
            "initial" => return Ok(ComputedValue::Initial),
            "inherit" => return Ok(ComputedValue::Inherit),
            "unset" => return Ok(ComputedValue::Unset),
            "revert" => return Ok(ComputedValue::Revert),
            _ => {}
        }

        if trimmed.starts_with('#') {
            return Color::from_hex(trimmed)
                .map(ComputedValue::Color)
                .ok_or_else(|| {
                    ComputedStyleError::ValueResolution(format!("Invalid hex color: {}", trimmed))
                });
        }

        if trimmed.starts_with("rgb(") || trimmed.starts_with("rgba(") {
            return self.parse_color_function(trimmed);
        }

        if trimmed.starts_with("url(") {
            let url = trimmed
                .trim_start_matches("url(")
                .trim_end_matches(')')
                .trim_matches('"')
                .trim_matches('\'');
            return Ok(ComputedValue::Url(url.to_string()));
        }

        if let Some((number, unit)) = self.parse_number_with_unit(trimmed) {
            return Ok(match unit {
                Some(_) if trimmed.ends_with('%') => ComputedValue::Percentage(number),
                Some(_) => ComputedValue::Length(number),
                None if number.fract() == 0.0 && !trimmed.contains('.') => {
                    ComputedValue::Integer(number as i32)
                }
                None => ComputedValue::Number(number),
            });
        }

        if trimmed.contains('(') && trimmed.ends_with(')') {
            return self.parse_function(trimmed);
        }

        if trimmed.contains(' ') {
            let items: Result<Vec<_>> = trimmed
                .split_whitespace()
                .map(|item| self.parse_raw(item, false))
                .collect();
            return Ok(ComputedValue::List(items?));
        }

        Ok(ComputedValue::Keyword(trimmed.to_string()))
    }

    fn extract_unit(&self, value: &str) -> Option<CSSUnit> {
        for (&unit, &unit_str) in UNIT_STRINGS.iter() {
            if value.ends_with(unit_str) {
                return Some(unit);
            }
        }
        None
    }
}

impl ComputedStyles {
    pub fn new(context: LayoutContext) -> Self {
        let styles = Self {
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
        let styles = Self {
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

    fn initialize_defaults(&self) {
        for prop in DEFAULT_PROPERTIES.iter() {
            self.properties
                .insert(prop.name.to_string(), prop.initial_value.clone());
        }
    }

    fn inherit_from_parent(&self) {
        if let Some(ref parent) = self.parent_styles {
            for &property_name in INHERITED_PROPERTIES {
                if let Some(value) = parent.properties.get(property_name) {
                    self.properties
                        .insert(property_name.to_string(), value.clone());
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
            Err(ComputedStyleError::PropertyComputation(format!(
                "Property not found: {}",
                name
            )))
        }
    }

    fn resolve_computed_value(
        &self,
        property_name: &str,
        value: &ComputedValue,
    ) -> Result<ComputedValue> {
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
                let resolved_vals: Result<Vec<_>> = vals
                    .iter()
                    .map(|v| self.resolve_computed_value(property_name, v))
                    .collect();
                Ok(ComputedValue::List(resolved_vals?))
            }
            _ => Ok(value.clone()),
        }
    }

    #[inline]
    fn get_percentage_base(&self, property_name: &str) -> PercentageBase {
        match property_name {
            "width" | "min-width" | "max-width" | "left" | "right" | "margin-left"
            | "margin-right" | "padding-left" | "padding-right" | "border-left-width"
            | "border-right-width" => PercentageBase::Width,

            "height"
            | "min-height"
            | "max-height"
            | "top"
            | "bottom"
            | "margin-top"
            | "margin-bottom"
            | "padding-top"
            | "padding-bottom"
            | "border-top-width"
            | "border-bottom-width" => PercentageBase::Height,

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
                    "Cannot resolve percentage without base".into(),
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
        let definition = Self::get_property_definition(property_name).ok_or_else(|| {
            ComputedStyleError::PropertyComputation(format!("Unknown property: {}", property_name))
        })?;
        Ok(definition.initial_value.clone())
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
            "min" => self.resolve_min_max_function(args, true),
            "max" => self.resolve_min_max_function(args, false),
            "clamp" => self.resolve_clamp_function(args),
            "var" => self.resolve_var_function(args),
            "rgb" => self.resolve_rgb_function(args),
            "rgba" => self.resolve_rgba_function(args),
            "hsl" => self.resolve_hsl_function(args),
            "hsla" => self.resolve_hsla_function(args),
            _ => Ok(ComputedValue::Function {
                name: name.to_string(),
                args: args.to_vec(),
            }),
        }
    }

    fn resolve_calc_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 1 {
            return Err(ComputedStyleError::ValueResolution(
                "calc() requires exactly one argument".into(),
            ));
        }

        if let ComputedValue::String(expr) = &args[0] {
            self.evaluate_calc_expression(expr)
        } else {
            Err(ComputedStyleError::ValueResolution(
                "Invalid calc() argument type".into(),
            ))
        }
    }

    fn evaluate_calc_expression(&self, expression: &str) -> Result<ComputedValue> {
        let trimmed = expression.trim();

        if let Some(pos) = trimmed.find('+') {
            let (left_str, right_str) = trimmed.split_at(pos);
            let left = self.parse_calc_value(left_str.trim())?;
            let right = self.parse_calc_value(right_str[1..].trim())?;
            return self.add_calc_values(&left, &right);
        }

        if let Some(pos) = trimmed.rfind('-') {
            if pos > 0 {
                let (left_str, right_str) = trimmed.split_at(pos);
                let left = self.parse_calc_value(left_str.trim())?;
                let right = self.parse_calc_value(right_str[1..].trim())?;
                return self.subtract_calc_values(&left, &right);
            }
        }

        self.parse_calc_value(trimmed)
    }

    fn parse_calc_value(&self, value: &str) -> Result<ComputedValue> {
        let v = value.trim();

        if let Some(num_str) = v.strip_suffix("px") {
            let num = num_str.parse::<f32>().map_err(|_| {
                ComputedStyleError::ValueResolution(format!(
                    "Invalid number in calc(): {}",
                    num_str
                ))
            })?;
            Ok(ComputedValue::Length(num))
        } else if let Some(num_str) = v.strip_suffix('%') {
            let num = num_str.parse::<f32>().map_err(|_| {
                ComputedStyleError::ValueResolution(format!(
                    "Invalid percentage in calc(): {}",
                    num_str
                ))
            })?;
            Ok(ComputedValue::Percentage(num))
        } else if let Some(num_str) = v.strip_suffix("em") {
            let num = num_str.parse::<f32>().map_err(|_| {
                ComputedStyleError::ValueResolution(format!(
                    "Invalid em value in calc(): {}",
                    num_str
                ))
            })?;
            Ok(ComputedValue::Length(num * self.context.font_size))
        } else {
            let num = v.parse::<f32>().map_err(|_| {
                ComputedStyleError::ValueResolution(format!("Invalid number in calc(): {}", v))
            })?;
            Ok(ComputedValue::Number(num))
        }
    }

    fn add_calc_values(
        &self,
        left: &ComputedValue,
        right: &ComputedValue,
    ) -> Result<ComputedValue> {
        use ComputedValue::*;
        match (left, right) {
            (Length(a), Length(b)) => Ok(Length(a + b)),
            (Number(a), Number(b)) => Ok(Number(a + b)),
            (Percentage(a), Percentage(b)) => Ok(Percentage(a + b)),
            _ => Err(ComputedStyleError::ValueResolution(
                "Incompatible types in calc() addition".into(),
            )),
        }
    }

    fn subtract_calc_values(
        &self,
        left: &ComputedValue,
        right: &ComputedValue,
    ) -> Result<ComputedValue> {
        use ComputedValue::*;
        match (left, right) {
            (Length(a), Length(b)) => Ok(Length(a - b)),
            (Number(a), Number(b)) => Ok(Number(a - b)),
            (Percentage(a), Percentage(b)) => Ok(Percentage(a - b)),
            _ => Err(ComputedStyleError::ValueResolution(
                "Incompatible types in calc() subtraction".into(),
            )),
        }
    }

    fn resolve_min_max_function(
        &self,
        args: &[ComputedValue],
        is_min: bool,
    ) -> Result<ComputedValue> {
        if args.is_empty() {
            return Err(ComputedStyleError::ValueResolution(format!(
                "{}() requires at least one argument",
                if is_min { "min" } else { "max" }
            )));
        }

        let mut result_val: Option<f32> = None;

        for arg in args {
            let resolved = self.resolve_computed_value("", arg)?;
            if let ComputedValue::Length(val) = resolved {
                result_val = Some(match result_val {
                    None => val,
                    Some(current) => {
                        if is_min {
                            current.min(val)
                        } else {
                            current.max(val)
                        }
                    }
                });
            } else {
                return Err(ComputedStyleError::ValueResolution(format!(
                    "{}() arguments must be length values",
                    if is_min { "min" } else { "max" }
                )));
            }
        }

        Ok(ComputedValue::Length(result_val.unwrap()))
    }

    fn resolve_clamp_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 3 {
            return Err(ComputedStyleError::ValueResolution(
                "clamp() requires exactly 3 arguments".into(),
            ));
        }

        let min_val = self.resolve_computed_value("", &args[0])?;
        let preferred_val = self.resolve_computed_value("", &args[1])?;
        let max_val = self.resolve_computed_value("", &args[2])?;

        match (min_val, preferred_val, max_val) {
            (
                ComputedValue::Length(min),
                ComputedValue::Length(pref),
                ComputedValue::Length(max),
            ) => Ok(ComputedValue::Length(pref.clamp(min, max))),
            _ => Err(ComputedStyleError::ValueResolution(
                "clamp() arguments must be length values".into(),
            )),
        }
    }

    fn resolve_var_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.is_empty() || args.len() > 2 {
            return Err(ComputedStyleError::ValueResolution(
                "var() requires 1 or 2 arguments".into(),
            ));
        }

        if let ComputedValue::String(var_name) = &args[0] {
            let custom_prop_name = format!("--{}", var_name);

            if let Some(custom_value) = self.properties.get(&custom_prop_name) {
                return Ok(custom_value.clone());
            }

            if args.len() == 2 {
                return Ok(args[1].clone());
            }
        }

        Err(ComputedStyleError::ValueResolution(
            "var() resolution failed".into(),
        ))
    }

    fn resolve_rgb_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 3 {
            return Err(ComputedStyleError::ValueResolution(
                "rgb() requires exactly 3 arguments".into(),
            ));
        }

        let r = self.resolve_color_component(&args[0])?;
        let g = self.resolve_color_component(&args[1])?;
        let b = self.resolve_color_component(&args[2])?;

        Ok(ComputedValue::Color(Color::from_rgb(r, g, b)))
    }

    fn resolve_rgba_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 4 {
            return Err(ComputedStyleError::ValueResolution(
                "rgba() requires exactly 4 arguments".into(),
            ));
        }

        let r = self.resolve_color_component(&args[0])?;
        let g = self.resolve_color_component(&args[1])?;
        let b = self.resolve_color_component(&args[2])?;
        let a = self.resolve_alpha_component(&args[3])?;

        Ok(ComputedValue::Color(Color::from_rgba(r, g, b, a)))
    }

    fn resolve_hsl_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 3 {
            return Err(ComputedStyleError::ValueResolution(
                "hsl() requires exactly 3 arguments".into(),
            ));
        }

        let h = self.resolve_hue_component(&args[0])?;
        let s = self.resolve_saturation_component(&args[1])?;
        let l = self.resolve_lightness_component(&args[2])?;

        Ok(ComputedValue::Color(Color::from_hsl(h, s, l)))
    }

    fn resolve_hsla_function(&self, args: &[ComputedValue]) -> Result<ComputedValue> {
        if args.len() != 4 {
            return Err(ComputedStyleError::ValueResolution(
                "hsla() requires exactly 4 arguments".into(),
            ));
        }

        let mut color = match self.resolve_hsl_function(&args[..3])? {
            ComputedValue::Color(c) => c,
            _ => {
                return Err(ComputedStyleError::ValueResolution(
                    "hsla() internal error".into(),
                ))
            }
        };

        color.a = self.resolve_alpha_component(&args[3])?;
        Ok(ComputedValue::Color(color))
    }

    fn resolve_color_component(&self, value: &ComputedValue) -> Result<u8> {
        match value {
            ComputedValue::Number(n) => Ok(*n as u8),
            ComputedValue::Percentage(p) => Ok((p * 2.55) as u8),
            _ => Err(ComputedStyleError::ValueResolution(
                "Invalid color component type".into(),
            )),
        }
    }

    fn resolve_alpha_component(&self, value: &ComputedValue) -> Result<f32> {
        match value {
            ComputedValue::Number(n) => Ok(n.clamp(0.0, 1.0)),
            ComputedValue::Percentage(p) => Ok((p * 0.01).clamp(0.0, 1.0)),
            _ => Err(ComputedStyleError::ValueResolution(
                "Invalid alpha component type".into(),
            )),
        }
    }

    fn resolve_hue_component(&self, value: &ComputedValue) -> Result<f32> {
        match value {
            ComputedValue::Number(n) => Ok(n.rem_euclid(360.0)),
            _ => Err(ComputedStyleError::ValueResolution(
                "Invalid hue component type".into(),
            )),
        }
    }

    fn resolve_saturation_component(&self, value: &ComputedValue) -> Result<f32> {
        match value {
            ComputedValue::Percentage(p) => Ok((p * 0.01).clamp(0.0, 1.0)),
            _ => Err(ComputedStyleError::ValueResolution(
                "Invalid saturation component type".into(),
            )),
        }
    }

    fn resolve_lightness_component(&self, value: &ComputedValue) -> Result<f32> {
        match value {
            ComputedValue::Percentage(p) => Ok((p * 0.01).clamp(0.0, 1.0)),
            _ => Err(ComputedStyleError::ValueResolution(
                "Invalid lightness component type".into(),
            )),
        }
    }

    fn parse_number_with_unit(&self, value: &str) -> Option<(f32, Option<CSSUnit>)> {
        let unit = self.extract_unit(value);

        let number_part = if let Some(unit) = unit {
            let unit_str = UNIT_STRINGS.get(&unit)?;
            value.trim_end_matches(unit_str)
        } else {
            value
        };

        number_part.parse::<f32>().ok().map(|n| (n, unit))
    }

    fn parse_color_function(&self, value: &str) -> Result<ComputedValue> {
        if value.starts_with("rgb(") {
            let content = value.trim_start_matches("rgb(").trim_end_matches(')');
            let parts: Vec<&str> = content.split(',').map(|s| s.trim()).collect();

            if parts.len() == 3 {
                let r = parts[0].parse::<u8>().map_err(|_| {
                    ComputedStyleError::ValueResolution("Invalid red component".into())
                })?;
                let g = parts[1].parse::<u8>().map_err(|_| {
                    ComputedStyleError::ValueResolution("Invalid green component".into())
                })?;
                let b = parts[2].parse::<u8>().map_err(|_| {
                    ComputedStyleError::ValueResolution("Invalid blue component".into())
                })?;
                return Ok(ComputedValue::Color(Color::from_rgb(r, g, b)));
            }
        } else if value.starts_with("rgba(") {
            let content = value.trim_start_matches("rgba(").trim_end_matches(')');
            let parts: Vec<&str> = content.split(',').map(|s| s.trim()).collect();

            if parts.len() == 4 {
                let r = parts[0].parse::<u8>().map_err(|_| {
                    ComputedStyleError::ValueResolution("Invalid red component".into())
                })?;
                let g = parts[1].parse::<u8>().map_err(|_| {
                    ComputedStyleError::ValueResolution("Invalid green component".into())
                })?;
                let b = parts[2].parse::<u8>().map_err(|_| {
                    ComputedStyleError::ValueResolution("Invalid blue component".into())
                })?;
                let a = parts[3].parse::<f32>().map_err(|_| {
                    ComputedStyleError::ValueResolution("Invalid alpha component".into())
                })?;
                return Ok(ComputedValue::Color(Color::from_rgba(r, g, b, a)));
            }
        }

        Err(ComputedStyleError::ValueResolution(format!(
            "Invalid color function: {}",
            value
        )))
    }

    fn parse_function(&self, value: &str) -> Result<ComputedValue> {
        let open_paren = value
            .find('(')
            .ok_or_else(|| ComputedStyleError::ValueResolution("Invalid function syntax".into()))?;
        let name = value[..open_paren].trim();
        let args_str = value[open_paren + 1..value.len() - 1].trim();

        let mut args = Vec::new();
        if !args_str.is_empty() {
            for arg in args_str.split(',') {
                args.push(self.parse_raw(arg.trim(), false)?);
            }
        }

        Ok(ComputedValue::Function {
            name: name.to_string(),
            args,
        })
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

    pub fn get_all_properties(&self) -> HashMap<String, ComputedValue> {
        self.properties
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    #[inline]
    fn is_inherited_property(name: &str) -> bool {
        INHERITED_PROPERTIES.binary_search(&name).is_ok()
    }

    fn get_property_definition(name: &str) -> Option<&PropertyDefinition> {
        DEFAULT_PROPERTIES.iter().find(|def| def.name == name)
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

        if let Some(root_node) = document.get_root_node() {
            let context = self.get_current_context();
            self.compute_styles_recursive(root_node, None, document, context)?;
        }

        Ok(())
    }

    fn compute_styles_recursive(
        &self,
        node: NodeId,
        parent_styles: Option<Arc<ComputedStyles>>,
        document: &Document,
        context: LayoutContext,
    ) -> Result<()> {
        let computed_styles = if let Some(parent) = parent_styles {
            Arc::new(ComputedStyles::with_parent(context.clone(), parent))
        } else {
            Arc::new(ComputedStyles::new(context.clone()))
        };

        self.apply_matching_rules(node, &computed_styles, document)?;
        self.style_cache.insert(node, computed_styles.clone());

        for &child_node in &document.get_children(node) {
            self.compute_styles_recursive(
                child_node,
                Some(computed_styles.clone()),
                document,
                context.clone(),
            )?;
        }

        Ok(())
    }

    fn apply_matching_rules(
        &self,
        node: NodeId,
        computed_styles: &ComputedStyles,
        document: &Document,
    ) -> Result<()> {
        let stylesheet_cache = self.stylesheet_cache.read();

        for rule_arc in stylesheet_cache.iter() {
            match rule_arc.as_ref() {
                CSSRule::Style(style_rule) => {
                    self.try_apply_style_rule(node, style_rule, computed_styles, document)?;
                }
                CSSRule::Media(media_rule) => {
                    if self.evaluate_media_query(&media_rule.media_query) {
                        self.apply_media_rule_styles(node, media_rule, computed_styles, document)?;
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn try_apply_style_rule(
        &self,
        node: NodeId,
        style_rule: &CSSStyleRule,
        computed_styles: &ComputedStyles,
        document: &Document,
    ) -> Result<()> {
        for selector in &style_rule.selectors {
            let selector_str = format!("{:?}", selector);

            if self
                .selector_engine
                .matches(&selector_str, node, document)
                .map_err(|e| ComputedStyleError::Cascade(e.to_string()))?
            {
                self.apply_declarations_from_style_rule(style_rule, computed_styles)?;
                break;
            }
        }
        Ok(())
    }

    fn apply_media_rule_styles(
        &self,
        node: NodeId,
        media_rule: &CSSMediaRule,
        computed_styles: &ComputedStyles,
        document: &Document,
    ) -> Result<()> {
        for nested_rule in &media_rule.rules {
            if let CSSRule::Style(style_rule) = nested_rule {
                self.try_apply_style_rule(node, style_rule, computed_styles, document)?;
            }
        }
        Ok(())
    }

    fn apply_declarations_from_style_rule(
        &self,
        style_rule: &CSSStyleRule,
        computed_styles: &ComputedStyles,
    ) -> Result<()> {
        for (property_name, property_value, is_important) in &style_rule.declarations.properties {
            let computed_value = computed_styles.parse_raw(property_value, *is_important)?;

            let effective_specificity = if *is_important {
                style_rule.specificity + 1000
            } else {
                style_rule.specificity
            };

            computed_styles.set_property(
                property_name,
                computed_value,
                effective_specificity,
                "author",
            );
        }

        Ok(())
    }

    fn evaluate_media_query(&self, _media_query: &crate::core::css::parser::MediaQuery) -> bool {
        true
    }

    pub fn get_computed_styles(&self, node: NodeId) -> Option<Arc<ComputedStyles>> {
        self.style_cache.get(&node).map(|entry| entry.clone())
    }

    pub fn add_stylesheet(&self, rules: Vec<CSSRule>) {
        let mut stylesheet_cache = self.stylesheet_cache.write();
        stylesheet_cache.extend(rules.into_iter().map(Arc::new));
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
        self.context_stack
            .read()
            .last()
            .cloned()
            .unwrap_or_default()
    }

    pub fn push_context(&self, context: LayoutContext) {
        self.context_stack.write().push(context);
    }

    pub fn pop_context(&self) {
        let mut context_stack = self.context_stack.write();
        if context_stack.len() > 1 {
            context_stack.pop();
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

impl Default for StyleEngine {
    fn default() -> Self {
        Self::new()
    }
}
