pub mod computed;
pub mod parser;
pub mod selector;

pub use computed::{ComputedStyles, StyleEngine};
pub use parser::{CSSParser, CSSRule, CSSStyleRule, CSSMediaRule, CSSImportRule, CSSFontFaceRule, CSSKeyframesRule, CSSKeyframeRule};
pub use selector::{SelectorEngine, Selector, SelectorMatcher, Specificity};

use std::sync::Arc;
use std::collections::HashMap;
use parking_lot::{RwLock, Mutex};
use dashmap::DashMap;
use smallvec::SmallVec;
use thiserror::Error;
use serde::{Serialize, Deserialize};

use crate::core::dom::{Document, NodeId, Element};

#[derive(Error, Debug)]
pub enum CSSError {
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Invalid property: {0}")]
    InvalidProperty(String),
    #[error("Invalid value: {0}")]
    InvalidValue(String),
    #[error("Selector error: {0}")]
    Selector(String),
    #[error("Media query error: {0}")]
    MediaQuery(String),
    #[error("Animation error: {0}")]
    Animation(String),
}

pub type Result<T> = std::result::Result<T, CSSError>;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CSSValue {
    pub raw: String,
    pub computed: ComputedValue,
    pub unit: Option<CSSUnit>,
    pub important: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ComputedValue {
    Length(f32),
    Percentage(f32),
    Number(f32),
    Integer(i32),
    String(String),
    Keyword(String),
    Color(Color),
    Url(String),
    Function {
        name: String,
        args: Vec<ComputedValue>,
    },
    List(Vec<ComputedValue>),
    None,
    Auto,
    Initial,
    Inherit,
    Unset,
    Revert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CSSUnit {
    Px,
    Em,
    Rem,
    Vh,
    Vw,
    Vmin,
    Vmax,
    Percent,
    Pt,
    Pc,
    In,
    Cm,
    Mm,
    Ex,
    Ch,
    Q,
    Deg,
    Rad,
    Grad,
    Turn,
    S,
    Ms,
    Hz,
    Khz,
    Dpi,
    Dpcm,
    Dppx,
    Fr,
}

impl CSSUnit {
    pub fn is_length(&self) -> bool {
        matches!(self, 
            CSSUnit::Px | CSSUnit::Em | CSSUnit::Rem | 
            CSSUnit::Pt | CSSUnit::Pc | CSSUnit::In |
            CSSUnit::Cm | CSSUnit::Mm | CSSUnit::Ex | CSSUnit::Ch | CSSUnit::Q
        )
    }

    pub fn is_viewport(&self) -> bool {
        matches!(self, CSSUnit::Vh | CSSUnit::Vw | CSSUnit::Vmin | CSSUnit::Vmax)
    }

    pub fn is_angle(&self) -> bool {
        matches!(self, CSSUnit::Deg | CSSUnit::Rad | CSSUnit::Grad | CSSUnit::Turn)
    }

    pub fn is_time(&self) -> bool {
        matches!(self, CSSUnit::S | CSSUnit::Ms)
    }

    pub fn is_frequency(&self) -> bool {
        matches!(self, CSSUnit::Hz | CSSUnit::Khz)
    }

    pub fn is_resolution(&self) -> bool {
        matches!(self, CSSUnit::Dpi | CSSUnit::Dpcm | CSSUnit::Dppx)
    }

    pub fn to_pixels(&self, value: f32, context: &LayoutContext) -> f32 {
        match self {
            CSSUnit::Px => value,
            CSSUnit::Em => value * context.font_size,
            CSSUnit::Rem => value * context.root_font_size,
            CSSUnit::Vh => value * context.viewport_height / 100.0,
            CSSUnit::Vw => value * context.viewport_width / 100.0,
            CSSUnit::Vmin => value * context.viewport_width.min(context.viewport_height) / 100.0,
            CSSUnit::Vmax => value * context.viewport_width.max(context.viewport_height) / 100.0,
            CSSUnit::Percent => value * context.containing_block_width / 100.0,
            CSSUnit::Pt => value * 4.0 / 3.0,
            CSSUnit::Pc => value * 16.0,
            CSSUnit::In => value * 96.0,
            CSSUnit::Cm => value * 96.0 / 2.54,
            CSSUnit::Mm => value * 96.0 / 25.4,
            CSSUnit::Q => value * 96.0 / 101.6,
            CSSUnit::Ex => value * context.font_size * 0.5,
            CSSUnit::Ch => value * context.font_size * 0.5,
            _ => value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

impl Color {
    pub fn new(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim_start_matches('#');
        
        match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                Some(Self::new(r, g, b, 1.0))
            },
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(Self::new(r, g, b, 1.0))
            },
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()? as f32 / 255.0;
                Some(Self::new(r, g, b, a))
            },
            _ => None,
        }
    }

    pub fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self::new(r, g, b, 1.0)
    }

    pub fn from_rgba(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self::new(r, g, b, a.clamp(0.0, 1.0))
    }

    pub fn from_hsl(h: f32, s: f32, l: f32) -> Self {
        let h = h % 360.0;
        let s = s.clamp(0.0, 1.0);
        let l = l.clamp(0.0, 1.0);

        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let m = l - c / 2.0;

        let (r_prime, g_prime, b_prime) = if h < 60.0 {
            (c, x, 0.0)
        } else if h < 120.0 {
            (x, c, 0.0)
        } else if h < 180.0 {
            (0.0, c, x)
        } else if h < 240.0 {
            (0.0, x, c)
        } else if h < 300.0 {
            (x, 0.0, c)
        } else {
            (c, 0.0, x)
        };

        let r = ((r_prime + m) * 255.0) as u8;
        let g = ((g_prime + m) * 255.0) as u8;
        let b = ((b_prime + m) * 255.0) as u8;

        Self::new(r, g, b, 1.0)
    }

    pub fn to_hex(&self) -> String {
        if self.a < 1.0 {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, (self.a * 255.0) as u8)
        } else {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        }
    }

    pub fn to_rgb(&self) -> String {
        format!("rgb({}, {}, {})", self.r, self.g, self.b)
    }

    pub fn to_rgba(&self) -> String {
        format!("rgba({}, {}, {}, {})", self.r, self.g, self.b, self.a)
    }

    pub fn luminance(&self) -> f32 {
        let r = self.r as f32 / 255.0;
        let g = self.g as f32 / 255.0;
        let b = self.b as f32 / 255.0;

        let r_linear = if r <= 0.03928 { r / 12.92 } else { ((r + 0.055) / 1.055).powf(2.4) };
        let g_linear = if g <= 0.03928 { g / 12.92 } else { ((g + 0.055) / 1.055).powf(2.4) };
        let b_linear = if b <= 0.03928 { b / 12.92 } else { ((b + 0.055) / 1.055).powf(2.4) };

        0.2126 * r_linear + 0.7152 * g_linear + 0.0722 * b_linear
    }

    pub fn contrast_ratio(&self, other: &Color) -> f32 {
        let l1 = self.luminance();
        let l2 = other.luminance();
        
        let lighter = l1.max(l2);
        let darker = l1.min(l2);
        
        (lighter + 0.05) / (darker + 0.05)
    }

    pub fn mix(&self, other: &Color, ratio: f32) -> Self {
        let ratio = ratio.clamp(0.0, 1.0);
        let inv_ratio = 1.0 - ratio;

        Self::new(
            (self.r as f32 * inv_ratio + other.r as f32 * ratio) as u8,
            (self.g as f32 * inv_ratio + other.g as f32 * ratio) as u8,
            (self.b as f32 * inv_ratio + other.b as f32 * ratio) as u8,
            self.a * inv_ratio + other.a * ratio,
        )
    }

    pub const BLACK: Color = Color { r: 0, g: 0, b: 0, a: 1.0 };
    pub const WHITE: Color = Color { r: 255, g: 255, b: 255, a: 1.0 };
    pub const RED: Color = Color { r: 255, g: 0, b: 0, a: 1.0 };
    pub const GREEN: Color = Color { r: 0, g: 255, b: 0, a: 1.0 };
    pub const BLUE: Color = Color { r: 0, g: 0, b: 255, a: 1.0 };
    pub const TRANSPARENT: Color = Color { r: 0, g: 0, b: 0, a: 0.0 };
}

#[derive(Debug, Clone)]
pub struct CSSStyleDeclaration {
    properties: Arc<DashMap<String, CSSValue>>,
    parent_rule: Option<Arc<CSSRule>>,
    css_text: Arc<RwLock<String>>,
    length: Arc<RwLock<usize>>,
}

impl CSSStyleDeclaration {
    pub fn new() -> Self {
        Self {
            properties: Arc::new(DashMap::new()),
            parent_rule: None,
            css_text: Arc::new(RwLock::new(String::new())),
            length: Arc::new(RwLock::new(0)),
        }
    }

    pub fn with_parent_rule(parent_rule: Arc<CSSRule>) -> Self {
        Self {
            properties: Arc::new(DashMap::new()),
            parent_rule: Some(parent_rule),
            css_text: Arc::new(RwLock::new(String::new())),
            length: Arc::new(RwLock::new(0)),
        }
    }

    pub fn get_property_value(&self, property: &str) -> Option<String> {
        self.properties.get(property).map(|entry| entry.raw.clone())
    }

    pub fn get_property_priority(&self, property: &str) -> String {
        self.properties.get(property)
            .map(|entry| if entry.important { "important".to_string() } else { String::new() })
            .unwrap_or_default()
    }

    pub fn set_property(&self, property: &str, value: &str, priority: &str) -> Result<()> {
        let important = priority == "important";
        
        let css_value = CSSValue {
            raw: value.to_string(),
            computed: self.parse_value(value)?,
            unit: self.parse_unit(value),
            important,
        };

        self.properties.insert(property.to_string(), css_value);
        
        {
            let mut length = self.length.write();
            if !self.properties.contains_key(property) {
                *length += 1;
            }
        }

        self.update_css_text();
        Ok(())
    }

    pub fn remove_property(&self, property: &str) -> Result<String> {
        if let Some((_, css_value)) = self.properties.remove(property) {
            let mut length = self.length.write();
            *length = (*length).saturating_sub(1);
            
            self.update_css_text();
            Ok(css_value.raw)
        } else {
            Err(CSSError::InvalidProperty(property.to_string()))
        }
    }

    pub fn item(&self, index: usize) -> Option<String> {
        self.properties.iter()
            .nth(index)
            .map(|entry| entry.key().clone())
    }

    pub fn get_length(&self) -> usize {
        *self.length.read()
    }

    pub fn get_css_text(&self) -> String {
        self.css_text.read().clone()
    }

    pub fn set_css_text(&self, css_text: &str) -> Result<()> {
        self.properties.clear();
        *self.length.write() = 0;

        let parser = CSSParser::new();
        let declarations = parser.parse_declarations(css_text)?;

        for (property, value, important) in declarations {
            self.set_property(&property, &value, if important { "important" } else { "" })?;
        }

        Ok(())
    }

    fn parse_value(&self, value: &str) -> Result<ComputedValue> {
        let value = value.trim();

        if value.is_empty() {
            return Ok(ComputedValue::None);
        }

        match value {
            "auto" => Ok(ComputedValue::Auto),
            "initial" => Ok(ComputedValue::Initial),
            "inherit" => Ok(ComputedValue::Inherit),
            "unset" => Ok(ComputedValue::Unset),
            "revert" => Ok(ComputedValue::Revert),
            _ => {
                if value.starts_with('#') {
                    if let Some(color) = Color::from_hex(value) {
                        return Ok(ComputedValue::Color(color));
                    }
                }

                if value.starts_with("rgb(") || value.starts_with("rgba(") {
                    if let Some(color) = self.parse_color_function(value) {
                        return Ok(ComputedValue::Color(color));
                    }
                }

                if value.starts_with("url(") {
                    let url = value.trim_start_matches("url(").trim_end_matches(')');
                    let url = url.trim_matches('"').trim_matches('\'');
                    return Ok(ComputedValue::Url(url.to_string()));
                }

                if let Some((number, unit)) = self.parse_number_with_unit(value) {
                    return Ok(match unit {
                        Some(_) => ComputedValue::Length(number),
                        None => {
                            if number.fract() == 0.0 {
                                ComputedValue::Integer(number as i32)
                            } else {
                                ComputedValue::Number(number)
                            }
                        }
                    });
                }

                if value.ends_with('%') {
                    if let Ok(number) = value.trim_end_matches('%').parse::<f32>() {
                        return Ok(ComputedValue::Percentage(number));
                    }
                }

                if value.contains('(') && value.ends_with(')') {
                    return self.parse_function(value);
                }

                if value.contains(',') || value.contains(' ') {
                    return self.parse_list(value);
                }

                Ok(ComputedValue::Keyword(value.to_string()))
            }
        }
    }

    fn parse_unit(&self, value: &str) -> Option<CSSUnit> {
        let value = value.trim();
        
        if value.ends_with("px") { Some(CSSUnit::Px) }
        else if value.ends_with("em") { Some(CSSUnit::Em) }
        else if value.ends_with("rem") { Some(CSSUnit::Rem) }
        else if value.ends_with("vh") { Some(CSSUnit::Vh) }
        else if value.ends_with("vw") { Some(CSSUnit::Vw) }
        else if value.ends_with("vmin") { Some(CSSUnit::Vmin) }
        else if value.ends_with("vmax") { Some(CSSUnit::Vmax) }
        else if value.ends_with("%") { Some(CSSUnit::Percent) }
        else if value.ends_with("pt") { Some(CSSUnit::Pt) }
        else if value.ends_with("pc") { Some(CSSUnit::Pc) }
        else if value.ends_with("in") { Some(CSSUnit::In) }
        else if value.ends_with("cm") { Some(CSSUnit::Cm) }
        else if value.ends_with("mm") { Some(CSSUnit::Mm) }
        else if value.ends_with("ex") { Some(CSSUnit::Ex) }
        else if value.ends_with("ch") { Some(CSSUnit::Ch) }
        else if value.ends_with("q") { Some(CSSUnit::Q) }
        else if value.ends_with("deg") { Some(CSSUnit::Deg) }
        else if value.ends_with("rad") { Some(CSSUnit::Rad) }
        else if value.ends_with("grad") { Some(CSSUnit::Grad) }
        else if value.ends_with("turn") { Some(CSSUnit::Turn) }
        else if value.ends_with("s") { Some(CSSUnit::S) }
        else if value.ends_with("ms") { Some(CSSUnit::Ms) }
        else if value.ends_with("hz") { Some(CSSUnit::Hz) }
        else if value.ends_with("khz") { Some(CSSUnit::Khz) }
        else if value.ends_with("dpi") { Some(CSSUnit::Dpi) }
        else if value.ends_with("dpcm") { Some(CSSUnit::Dpcm) }
        else if value.ends_with("dppx") { Some(CSSUnit::Dppx) }
        else if value.ends_with("fr") { Some(CSSUnit::Fr) }
        else { None }
    }

    fn parse_number_with_unit(&self, value: &str) -> Option<(f32, Option<CSSUnit>)> {
        let unit = self.parse_unit(value);
        
        let number_part = if let Some(unit) = unit {
            let unit_str = match unit {
                CSSUnit::Px => "px",
                CSSUnit::Em => "em",
                CSSUnit::Rem => "rem",
                CSSUnit::Vh => "vh",
                CSSUnit::Vw => "vw",
                CSSUnit::Vmin => "vmin",
                CSSUnit::Vmax => "vmax",
                CSSUnit::Percent => "%",
                _ => "",
            };
            value.trim_end_matches(unit_str)
        } else {
            value
        };

        number_part.parse::<f32>().ok().map(|n| (n, unit))
    }

    fn parse_color_function(&self, value: &str) -> Option<Color> {
        if value.starts_with("rgb(") {
            let content = value.trim_start_matches("rgb(").trim_end_matches(')');
            let parts: Vec<&str> = content.split(',').map(|s| s.trim()).collect();
            
            if parts.len() == 3 {
                let r = parts[0].parse::<u8>().ok()?;
                let g = parts[1].parse::<u8>().ok()?;
                let b = parts[2].parse::<u8>().ok()?;
                return Some(Color::from_rgb(r, g, b));
            }
        } else if value.starts_with("rgba(") {
            let content = value.trim_start_matches("rgba(").trim_end_matches(')');
            let parts: Vec<&str> = content.split(',').map(|s| s.trim()).collect();
            
            if parts.len() == 4 {
                let r = parts[0].parse::<u8>().ok()?;
                let g = parts[1].parse::<u8>().ok()?;
                let b = parts[2].parse::<u8>().ok()?;
                let a = parts[3].parse::<f32>().ok()?;
                return Some(Color::from_rgba(r, g, b, a));
            }
        }
        
        None
    }

    fn parse_function(&self, value: &str) -> Result<ComputedValue> {
        let open_paren = value.find('(').ok_or_else(|| CSSError::Parse("Invalid function".to_string()))?;
        let name = value[..open_paren].trim();
        let args_str = value[open_paren + 1..value.len() - 1].trim();

        let mut args = Vec::new();
        if !args_str.is_empty() {
            for arg in args_str.split(',') {
                args.push(self.parse_value(arg.trim())?);
            }
        }

        Ok(ComputedValue::Function {
            name: name.to_string(),
            args,
        })
    }

    fn parse_list(&self, value: &str) -> Result<ComputedValue> {
        let items: Result<Vec<ComputedValue>> = value
            .split_whitespace()
            .map(|item| self.parse_value(item))
            .collect();
        
        Ok(ComputedValue::List(items?))
    }

    fn update_css_text(&self) {
        let mut css_text = String::new();
        
        for entry in self.properties.iter() {
            let property = entry.key();
            let value = entry.value();
            
            if !css_text.is_empty() {
                css_text.push_str("; ");
            }
            
            css_text.push_str(&format!("{}: {}", property, value.raw));
            
            if value.important {
                css_text.push_str(" !important");
            }
        }
        
        *self.css_text.write() = css_text;
    }

    pub fn get_computed_value(&self, property: &str) -> Option<ComputedValue> {
        self.properties.get(property).map(|entry| entry.computed.clone())
    }

    pub fn get_all_properties(&self) -> Vec<(String, CSSValue)> {
        self.properties.iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    pub fn has_property(&self, property: &str) -> bool {
        self.properties.contains_key(property)
    }

    pub fn clear(&self) {
        self.properties.clear();
        *self.length.write() = 0;
        self.update_css_text();
    }
}

#[derive(Debug, Clone)]
pub struct LayoutContext {
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub containing_block_width: f32,
    pub containing_block_height: f32,
    pub font_size: f32,
    pub root_font_size: f32,
    pub device_pixel_ratio: f32,
    pub writing_mode: WritingMode,
    pub direction: Direction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritingMode {
    HorizontalTb,
    VerticalRl,
    VerticalLr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Ltr,
    Rtl,
}

impl Default for LayoutContext {
    fn default() -> Self {
        Self {
            viewport_width: 1920.0,
            viewport_height: 1080.0,
            containing_block_width: 1920.0,
            containing_block_height: 1080.0,
            font_size: 16.0,
            root_font_size: 16.0,
            device_pixel_ratio: 1.0,
            writing_mode: WritingMode::HorizontalTb,
            direction: Direction::Ltr,
        }
    }
}