pub mod parser;

pub use parser::*;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub short_name: Option<String>,
    pub description: Option<String>,
    pub start_url: String,
    pub scope: Option<String>,
    pub display: DisplayMode,
    pub orientation: Option<Orientation>,
    pub theme_color: Option<String>,
    pub background_color: Option<String>,
    pub icons: Vec<Icon>,
    pub service_worker: Option<String>,
    pub categories: Vec<String>,
    pub lang: Option<String>,
    pub dir: Option<TextDirection>,
    pub shortcuts: Vec<Shortcut>,
    pub screenshots: Vec<Screenshot>,
    pub related_applications: Vec<RelatedApplication>,
    pub prefer_related_applications: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub enum DisplayMode {
    #[serde(rename = "fullscreen")]
    Fullscreen,
    #[serde(rename = "standalone")]
    #[default]
    Standalone,
    #[serde(rename = "minimal-ui")]
    MinimalUi,
    #[serde(rename = "browser")]
    Browser,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Orientation {
    #[serde(rename = "any")]
    Any,
    #[serde(rename = "natural")]
    Natural,
    #[serde(rename = "landscape")]
    Landscape,
    #[serde(rename = "portrait")]
    Portrait,
    #[serde(rename = "portrait-primary")]
    PortraitPrimary,
    #[serde(rename = "portrait-secondary")]
    PortraitSecondary,
    #[serde(rename = "landscape-primary")]
    LandscapePrimary,
    #[serde(rename = "landscape-secondary")]
    LandscapeSecondary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TextDirection {
    #[serde(rename = "ltr")]
    LeftToRight,
    #[serde(rename = "rtl")]
    RightToLeft,
    #[serde(rename = "auto")]
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Icon {
    pub src: String,
    pub sizes: Option<String>,
    #[serde(rename = "type")]
    pub icon_type: Option<String>,
    pub purpose: Option<IconPurpose>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub enum IconPurpose {
    #[serde(rename = "any")]
    #[default]
    Any,
    #[serde(rename = "maskable")]
    Maskable,
    #[serde(rename = "monochrome")]
    Monochrome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shortcut {
    pub name: String,
    pub short_name: Option<String>,
    pub description: Option<String>,
    pub url: String,
    pub icons: Vec<Icon>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Screenshot {
    pub src: String,
    pub sizes: Option<String>,
    #[serde(rename = "type")]
    pub screenshot_type: Option<String>,
    pub form_factor: Option<FormFactor>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FormFactor {
    #[serde(rename = "narrow")]
    Narrow,
    #[serde(rename = "wide")]
    Wide,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedApplication {
    pub platform: String,
    pub url: Option<String>,
    pub id: Option<String>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            name: "Untitled App".to_string(),
            short_name: None,
            description: None,
            start_url: "/".to_string(),
            scope: None,
            display: DisplayMode::Standalone,
            orientation: None,
            theme_color: None,
            background_color: None,
            icons: Vec::new(),
            service_worker: None,
            categories: Vec::new(),
            lang: None,
            dir: None,
            shortcuts: Vec::new(),
            screenshots: Vec::new(),
            related_applications: Vec::new(),
            prefer_related_applications: false,
        }
    }
}



#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("Invalid manifest: {0}")]
    InvalidManifest(String),
    #[error("Missing required field: {0}")]
    MissingField(String),
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
}
