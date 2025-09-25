pub mod computed;
pub mod parser;
pub mod selector;

pub use computed::{ComputedStyles, StyleEngine};
pub use parser::{
    CSSFontFaceRule, CSSImportRule, CSSKeyframeRule, CSSKeyframesRule, CSSMediaRule, CSSParser,
    CSSRule, CSSStyleRule, ParseError,
};
pub use selector::{Selector, SelectorEngine, SelectorMatcher, Specificity};

use dashmap::DashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

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

impl From<ParseError> for CSSError {
    fn from(err: ParseError) -> Self {
        CSSError::Parse(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, CSSError>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CSSValue {
    pub raw: String,
    pub computed: ComputedValue,
    pub unit: Option<CSSUnit>,
    pub important: bool,
}

impl CSSValue {
    pub fn new(
        raw: String,
        computed: ComputedValue,
        unit: Option<CSSUnit>,
        important: bool,
    ) -> Self {
        Self {
            raw,
            computed,
            unit,
            important,
        }
    }

    pub fn approximate_eq(&self, other: &Self) -> bool {
        self.raw == other.raw
            && self.computed.approximate_eq(&other.computed)
            && self.unit == other.unit
            && self.important == other.important
    }
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

impl ComputedValue {
    const EPSILON: f32 = 1e-6;

    pub fn approximate_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ComputedValue::Length(a), ComputedValue::Length(b)) => (a - b).abs() < Self::EPSILON,
            (ComputedValue::Percentage(a), ComputedValue::Percentage(b)) => {
                (a - b).abs() < Self::EPSILON
            }
            (ComputedValue::Number(a), ComputedValue::Number(b)) => (a - b).abs() < Self::EPSILON,
            (ComputedValue::Integer(a), ComputedValue::Integer(b)) => a == b,
            (ComputedValue::String(a), ComputedValue::String(b)) => a == b,
            (ComputedValue::Keyword(a), ComputedValue::Keyword(b)) => a == b,
            (ComputedValue::Color(a), ComputedValue::Color(b)) => a.approximate_eq(b),
            (ComputedValue::Url(a), ComputedValue::Url(b)) => a == b,
            (
                ComputedValue::Function { name: n1, args: a1 },
                ComputedValue::Function { name: n2, args: a2 },
            ) => {
                n1 == n2
                    && a1.len() == a2.len()
                    && a1.iter().zip(a2.iter()).all(|(x, y)| x.approximate_eq(y))
            }
            (ComputedValue::List(a), ComputedValue::List(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.approximate_eq(y))
            }
            _ => std::mem::discriminant(self) == std::mem::discriminant(other),
        }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            ComputedValue::Length(_)
                | ComputedValue::Percentage(_)
                | ComputedValue::Number(_)
                | ComputedValue::Integer(_)
        )
    }

    pub fn to_f32(&self) -> Option<f32> {
        match self {
            ComputedValue::Length(v) | ComputedValue::Percentage(v) | ComputedValue::Number(v) => {
                Some(*v)
            }
            ComputedValue::Integer(v) => Some(*v as f32),
            _ => None,
        }
    }
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
        matches!(
            self,
            CSSUnit::Px
                | CSSUnit::Em
                | CSSUnit::Rem
                | CSSUnit::Pt
                | CSSUnit::Pc
                | CSSUnit::In
                | CSSUnit::Cm
                | CSSUnit::Mm
                | CSSUnit::Ex
                | CSSUnit::Ch
                | CSSUnit::Q
        )
    }

    pub fn is_viewport(&self) -> bool {
        matches!(
            self,
            CSSUnit::Vh | CSSUnit::Vw | CSSUnit::Vmin | CSSUnit::Vmax
        )
    }

    pub fn is_angle(&self) -> bool {
        matches!(
            self,
            CSSUnit::Deg | CSSUnit::Rad | CSSUnit::Grad | CSSUnit::Turn
        )
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

    pub fn from_suffix(s: &str) -> Option<Self> {
        match s {
            "px" => Some(CSSUnit::Px),
            "em" => Some(CSSUnit::Em),
            "rem" => Some(CSSUnit::Rem),
            "vh" => Some(CSSUnit::Vh),
            "vw" => Some(CSSUnit::Vw),
            "vmin" => Some(CSSUnit::Vmin),
            "vmax" => Some(CSSUnit::Vmax),
            "%" => Some(CSSUnit::Percent),
            "pt" => Some(CSSUnit::Pt),
            "pc" => Some(CSSUnit::Pc),
            "in" => Some(CSSUnit::In),
            "cm" => Some(CSSUnit::Cm),
            "mm" => Some(CSSUnit::Mm),
            "ex" => Some(CSSUnit::Ex),
            "ch" => Some(CSSUnit::Ch),
            "q" => Some(CSSUnit::Q),
            "deg" => Some(CSSUnit::Deg),
            "rad" => Some(CSSUnit::Rad),
            "grad" => Some(CSSUnit::Grad),
            "turn" => Some(CSSUnit::Turn),
            "s" => Some(CSSUnit::S),
            "ms" => Some(CSSUnit::Ms),
            "hz" => Some(CSSUnit::Hz),
            "khz" => Some(CSSUnit::Khz),
            "dpi" => Some(CSSUnit::Dpi),
            "dpcm" => Some(CSSUnit::Dpcm),
            "dppx" => Some(CSSUnit::Dppx),
            "fr" => Some(CSSUnit::Fr),
            _ => None,
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
        Self {
            r,
            g,
            b,
            a: a.clamp(0.0, 1.0),
        }
    }

    pub fn approximate_eq(&self, other: &Self) -> bool {
        self.r == other.r
            && self.g == other.g
            && self.b == other.b
            && (self.a - other.a).abs() < 1e-6
    }

    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim_start_matches('#');

        match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                Some(Self::new(r, g, b, 1.0))
            }
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(Self::new(r, g, b, 1.0))
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()? as f32 / 255.0;
                Some(Self::new(r, g, b, a))
            }
            _ => None,
        }
    }

    pub fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self::new(r, g, b, 1.0)
    }

    pub fn from_rgba(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self::new(r, g, b, a)
    }

    pub fn from_hsl(h: f32, s: f32, l: f32) -> Self {
        let h = h % 360.0;
        let s = s.clamp(0.0, 1.0);
        let l = l.clamp(0.0, 1.0);

        if s == 0.0 {
            let gray = (l * 255.0) as u8;
            return Self::new(gray, gray, gray, 1.0);
        }

        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let m = l - c / 2.0;

        let (r_prime, g_prime, b_prime) = match h {
            h if h < 60.0 => (c, x, 0.0),
            h if h < 120.0 => (x, c, 0.0),
            h if h < 180.0 => (0.0, c, x),
            h if h < 240.0 => (0.0, x, c),
            h if h < 300.0 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };

        let r = ((r_prime + m) * 255.0) as u8;
        let g = ((g_prime + m) * 255.0) as u8;
        let b = ((b_prime + m) * 255.0) as u8;

        Self::new(r, g, b, 1.0)
    }

    pub fn to_hex(&self) -> String {
        if self.a < 1.0 {
            format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                self.r,
                self.g,
                self.b,
                (self.a * 255.0) as u8
            )
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
        let to_linear = |c: f32| {
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };

        let r = to_linear(self.r as f32 / 255.0);
        let g = to_linear(self.g as f32 / 255.0);
        let b = to_linear(self.b as f32 / 255.0);

        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    pub fn contrast_ratio(&self, other: &Color) -> f32 {
        let l1 = self.luminance();
        let l2 = other.luminance();
        let (lighter, darker) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
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

    pub const BLACK: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 1.0,
    };
    pub const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
        a: 1.0,
    };
    pub const RED: Color = Color {
        r: 255,
        g: 0,
        b: 0,
        a: 1.0,
    };
    pub const GREEN: Color = Color {
        r: 0,
        g: 255,
        b: 0,
        a: 1.0,
    };
    pub const BLUE: Color = Color {
        r: 0,
        g: 0,
        b: 255,
        a: 1.0,
    };
    pub const TRANSPARENT: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 0.0,
    };
}

#[derive(Debug)]
pub struct CSSStyleDeclaration {
    properties: DashMap<String, CSSValue>,
    #[allow(dead_code)]
    parent_rule: Option<Arc<CSSRule>>,
    css_text: RwLock<String>,
    length: RwLock<usize>,
}

impl CSSStyleDeclaration {
    pub fn new() -> Self {
        Self {
            properties: DashMap::new(),
            parent_rule: None,
            css_text: RwLock::new(String::new()),
            length: RwLock::new(0),
        }
    }

    pub fn with_parent_rule(parent_rule: Arc<CSSRule>) -> Self {
        Self {
            properties: DashMap::new(),
            parent_rule: Some(parent_rule),
            css_text: RwLock::new(String::new()),
            length: RwLock::new(0),
        }
    }

    pub fn get_property_value(&self, property: &str) -> Option<String> {
        self.properties.get(property).map(|entry| entry.raw.clone())
    }

    pub fn get_property_priority(&self, property: &str) -> String {
        self.properties
            .get(property)
            .map(|entry| {
                if entry.important {
                    "important".to_string()
                } else {
                    String::new()
                }
            })
            .unwrap_or_default()
    }

    pub fn set_property(&self, property: &str, value: &str, priority: &str) -> Result<()> {
        let important = priority == "important";
        let css_value = self.parse_css_value(value, important)?;

        let is_new = !self.properties.contains_key(property);
        self.properties.insert(property.to_string(), css_value);

        if is_new {
            *self.length.write() += 1;
        }

        self.update_css_text();
        Ok(())
    }

    pub fn remove_property(&self, property: &str) -> Result<String> {
        if let Some((_, css_value)) = self.properties.remove(property) {
            *self.length.write() = self.length.read().saturating_sub(1);
            self.update_css_text();
            Ok(css_value.raw)
        } else {
            Err(CSSError::InvalidProperty(property.to_string()))
        }
    }

    pub fn item(&self, index: usize) -> Option<String> {
        self.properties
            .iter()
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

    fn parse_css_value(&self, value: &str, important: bool) -> Result<CSSValue> {
        let value = value.trim();

        if value.is_empty() {
            return Ok(CSSValue::new(
                value.to_string(),
                ComputedValue::None,
                None,
                important,
            ));
        }

        let computed = self.parse_computed_value(value)?;
        let unit = self.extract_unit(value);

        Ok(CSSValue::new(value.to_string(), computed, unit, important))
    }

    fn parse_computed_value(&self, value: &str) -> Result<ComputedValue> {
        match value {
            "auto" => Ok(ComputedValue::Auto),
            "initial" => Ok(ComputedValue::Initial),
            "inherit" => Ok(ComputedValue::Inherit),
            "unset" => Ok(ComputedValue::Unset),
            "revert" => Ok(ComputedValue::Revert),
            _ => {
                if value.starts_with('#') {
                    return Color::from_hex(value)
                        .map(ComputedValue::Color)
                        .ok_or_else(|| {
                            CSSError::InvalidValue(format!("Invalid hex color: {}", value))
                        });
                }

                if value.starts_with("rgb(") || value.starts_with("rgba(") {
                    return self.parse_color_function(value);
                }

                if value.starts_with("url(") {
                    let url = value.trim_start_matches("url(").trim_end_matches(')');
                    let url = url.trim_matches('"').trim_matches('\'');
                    return Ok(ComputedValue::Url(url.to_string()));
                }

                if let Some((number, _)) = self.parse_number_with_unit(value) {
                    return Ok(if value.ends_with('%') {
                        ComputedValue::Percentage(number)
                    } else if number.fract() == 0.0 && !value.contains('.') {
                        ComputedValue::Integer(number as i32)
                    } else if self.extract_unit(value).is_some() {
                        ComputedValue::Length(number)
                    } else {
                        ComputedValue::Number(number)
                    });
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

    fn extract_unit(&self, value: &str) -> Option<CSSUnit> {
        for suffix_len in (1..=5).rev() {
            if value.len() > suffix_len {
                let suffix = &value[value.len() - suffix_len..];
                if let Some(unit) = CSSUnit::from_suffix(suffix) {
                    return Some(unit);
                }
            }
        }
        None
    }

    fn parse_number_with_unit(&self, value: &str) -> Option<(f32, Option<CSSUnit>)> {
        let unit = self.extract_unit(value);

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
                CSSUnit::Pt => "pt",
                CSSUnit::Pc => "pc",
                CSSUnit::In => "in",
                CSSUnit::Cm => "cm",
                CSSUnit::Mm => "mm",
                CSSUnit::Ex => "ex",
                CSSUnit::Ch => "ch",
                CSSUnit::Q => "q",
                CSSUnit::Deg => "deg",
                CSSUnit::Rad => "rad",
                CSSUnit::Grad => "grad",
                CSSUnit::Turn => "turn",
                CSSUnit::S => "s",
                CSSUnit::Ms => "ms",
                CSSUnit::Hz => "hz",
                CSSUnit::Khz => "khz",
                CSSUnit::Dpi => "dpi",
                CSSUnit::Dpcm => "dpcm",
                CSSUnit::Dppx => "dppx",
                CSSUnit::Fr => "fr",
            };
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
                let r = parts[0]
                    .parse::<u8>()
                    .map_err(|_| CSSError::InvalidValue("Invalid red component".to_string()))?;
                let g = parts[1]
                    .parse::<u8>()
                    .map_err(|_| CSSError::InvalidValue("Invalid green component".to_string()))?;
                let b = parts[2]
                    .parse::<u8>()
                    .map_err(|_| CSSError::InvalidValue("Invalid blue component".to_string()))?;
                return Ok(ComputedValue::Color(Color::from_rgb(r, g, b)));
            }
        } else if value.starts_with("rgba(") {
            let content = value.trim_start_matches("rgba(").trim_end_matches(')');
            let parts: Vec<&str> = content.split(',').map(|s| s.trim()).collect();

            if parts.len() == 4 {
                let r = parts[0]
                    .parse::<u8>()
                    .map_err(|_| CSSError::InvalidValue("Invalid red component".to_string()))?;
                let g = parts[1]
                    .parse::<u8>()
                    .map_err(|_| CSSError::InvalidValue("Invalid green component".to_string()))?;
                let b = parts[2]
                    .parse::<u8>()
                    .map_err(|_| CSSError::InvalidValue("Invalid blue component".to_string()))?;
                let a = parts[3]
                    .parse::<f32>()
                    .map_err(|_| CSSError::InvalidValue("Invalid alpha component".to_string()))?;
                return Ok(ComputedValue::Color(Color::from_rgba(r, g, b, a)));
            }
        }

        Err(CSSError::InvalidValue(format!(
            "Invalid color function: {}",
            value
        )))
    }

    fn parse_function(&self, value: &str) -> Result<ComputedValue> {
        let open_paren = value
            .find('(')
            .ok_or_else(|| CSSError::Parse("Invalid function".to_string()))?;
        let name = value[..open_paren].trim();
        let args_str = value[open_paren + 1..value.len() - 1].trim();

        let mut args = Vec::new();
        if !args_str.is_empty() {
            for arg in args_str.split(',') {
                args.push(self.parse_computed_value(arg.trim())?);
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
            .map(|item| self.parse_computed_value(item))
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
        self.properties
            .get(property)
            .map(|entry| entry.computed.clone())
    }

    pub fn get_all_properties(&self) -> Vec<(String, CSSValue)> {
        self.properties
            .iter()
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

impl Default for CSSStyleDeclaration {
    fn default() -> Self {
        Self::new()
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

impl LayoutContext {
    pub fn with_viewport(mut self, width: f32, height: f32) -> Self {
        self.viewport_width = width;
        self.viewport_height = height;
        self
    }

    pub fn with_containing_block(mut self, width: f32, height: f32) -> Self {
        self.containing_block_width = width;
        self.containing_block_height = height;
        self
    }

    pub fn with_font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    pub fn with_root_font_size(mut self, size: f32) -> Self {
        self.root_font_size = size;
        self
    }
}
