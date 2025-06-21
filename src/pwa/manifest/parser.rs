use super::{Manifest, ManifestError};
use serde_json::Value;
use url::Url;

pub struct ManifestParser;

impl ManifestParser {
    pub fn new() -> Self {
        Self
    }

    pub fn parse(&self, json_str: &str, base_url: Option<&str>) -> Result<Manifest, ManifestError> {
        let value: Value = serde_json::from_str(json_str)
            .map_err(|e| ManifestError::ParseError(e.to_string()))?;

        let mut manifest = Manifest::default();

        if let Some(name) = value.get("name").and_then(|v| v.as_str()) {
            manifest.name = name.to_string();
        } else {
            return Err(ManifestError::MissingField("name".to_string()));
        }

        if let Some(short_name) = value.get("short_name").and_then(|v| v.as_str()) {
            manifest.short_name = Some(short_name.to_string());
        }

        if let Some(description) = value.get("description").and_then(|v| v.as_str()) {
            manifest.description = Some(description.to_string());
        }

        if let Some(start_url) = value.get("start_url").and_then(|v| v.as_str()) {
            manifest.start_url = self.resolve_url(start_url, base_url)?;
        }

        if let Some(scope) = value.get("scope").and_then(|v| v.as_str()) {
            manifest.scope = Some(self.resolve_url(scope, base_url)?);
        }

        if let Some(display) = value.get("display").and_then(|v| v.as_str()) {
            manifest.display = self.parse_display_mode(display)?;
        }

        if let Some(orientation) = value.get("orientation").and_then(|v| v.as_str()) {
            manifest.orientation = Some(self.parse_orientation(orientation)?);
        }

        if let Some(theme_color) = value.get("theme_color").and_then(|v| v.as_str()) {
            manifest.theme_color = Some(theme_color.to_string());
        }

        if let Some(background_color) = value.get("background_color").and_then(|v| v.as_str()) {
            manifest.background_color = Some(background_color.to_string());
        }

        if let Some(icons) = value.get("icons").and_then(|v| v.as_array()) {
            manifest.icons = self.parse_icons(icons, base_url)?;
        }

        if let Some(service_worker_url) = value.get("serviceworker")
            .and_then(|sw| sw.get("src"))
            .and_then(|v| v.as_str()) {
            manifest.service_worker = Some(self.resolve_url(service_worker_url, base_url)?);
        }

        if let Some(categories) = value.get("categories").and_then(|v| v.as_array()) {
            manifest.categories = categories
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect();
        }

        if let Some(lang) = value.get("lang").and_then(|v| v.as_str()) {
            manifest.lang = Some(lang.to_string());
        }

        if let Some(dir) = value.get("dir").and_then(|v| v.as_str()) {
            manifest.dir = Some(self.parse_text_direction(dir)?);
        }

        if let Some(shortcuts) = value.get("shortcuts").and_then(|v| v.as_array()) {
            manifest.shortcuts = self.parse_shortcuts(shortcuts, base_url)?;
        }

        if let Some(screenshots) = value.get("screenshots").and_then(|v| v.as_array()) {
            manifest.screenshots = self.parse_screenshots(screenshots, base_url)?;
        }

        if let Some(related_apps) = value.get("related_applications").and_then(|v| v.as_array()) {
            manifest.related_applications = self.parse_related_applications(related_apps)?;
        }

        if let Some(prefer_related) = value.get("prefer_related_applications").and_then(|v| v.as_bool()) {
            manifest.prefer_related_applications = prefer_related;
        }

        self.validate_manifest(&manifest)?;
        Ok(manifest)
    }

    fn resolve_url(&self, url: &str, base_url: Option<&str>) -> Result<String, ManifestError> {
        if url.starts_with("http://") || url.starts_with("https://") {
            return Ok(url.to_string());
        }

        if let Some(base) = base_url {
            let base_url = Url::parse(base)
                .map_err(|e| ManifestError::InvalidUrl(e.to_string()))?;
            let resolved = base_url.join(url)
                .map_err(|e| ManifestError::InvalidUrl(e.to_string()))?;
            Ok(resolved.to_string())
        } else {
            Ok(url.to_string())
        }
    }

    fn parse_display_mode(&self, display: &str) -> Result<super::DisplayMode, ManifestError> {
        match display {
            "fullscreen" => Ok(super::DisplayMode::Fullscreen),
            "standalone" => Ok(super::DisplayMode::Standalone),
            "minimal-ui" => Ok(super::DisplayMode::MinimalUi),
            "browser" => Ok(super::DisplayMode::Browser),
            _ => Ok(super::DisplayMode::Standalone),
        }
    }

    fn parse_orientation(&self, orientation: &str) -> Result<super::Orientation, ManifestError> {
        match orientation {
            "any" => Ok(super::Orientation::Any),
            "natural" => Ok(super::Orientation::Natural),
            "landscape" => Ok(super::Orientation::Landscape),
            "portrait" => Ok(super::Orientation::Portrait),
            "portrait-primary" => Ok(super::Orientation::PortraitPrimary),
            "portrait-secondary" => Ok(super::Orientation::PortraitSecondary),
            "landscape-primary" => Ok(super::Orientation::LandscapePrimary),
            "landscape-secondary" => Ok(super::Orientation::LandscapeSecondary),
            _ => Ok(super::Orientation::Any),
        }
    }

    fn parse_text_direction(&self, dir: &str) -> Result<super::TextDirection, ManifestError> {
        match dir {
            "ltr" => Ok(super::TextDirection::LeftToRight),
            "rtl" => Ok(super::TextDirection::RightToLeft),
            "auto" => Ok(super::TextDirection::Auto),
            _ => Ok(super::TextDirection::Auto),
        }
    }

    fn parse_icons(&self, icons: &[Value], base_url: Option<&str>) -> Result<Vec<super::Icon>, ManifestError> {
        let mut result = Vec::new();

        for icon_value in icons {
            if let Some(src) = icon_value.get("src").and_then(|v| v.as_str()) {
                let icon = super::Icon {
                    src: self.resolve_url(src, base_url)?,
                    sizes: icon_value.get("sizes").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    icon_type: icon_value.get("type").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    purpose: icon_value.get("purpose")
                        .and_then(|v| v.as_str())
                        .and_then(|s| self.parse_icon_purpose(s).ok()),
                };
                result.push(icon);
            }
        }

        Ok(result)
    }

    fn parse_icon_purpose(&self, purpose: &str) -> Result<super::IconPurpose, ManifestError> {
        match purpose {
            "any" => Ok(super::IconPurpose::Any),
            "maskable" => Ok(super::IconPurpose::Maskable),
            "monochrome" => Ok(super::IconPurpose::Monochrome),
            _ => Ok(super::IconPurpose::Any),
        }
    }

    fn parse_shortcuts(&self, shortcuts: &[Value], base_url: Option<&str>) -> Result<Vec<super::Shortcut>, ManifestError> {
        let mut result = Vec::new();

        for shortcut_value in shortcuts {
            if let (Some(name), Some(url)) = (
                shortcut_value.get("name").and_then(|v| v.as_str()),
                shortcut_value.get("url").and_then(|v| v.as_str())
            ) {
                let icons = if let Some(icons_array) = shortcut_value.get("icons").and_then(|v| v.as_array()) {
                    self.parse_icons(icons_array, base_url)?
                } else {
                    Vec::new()
                };

                let shortcut = super::Shortcut {
                    name: name.to_string(),
                    short_name: shortcut_value.get("short_name").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    description: shortcut_value.get("description").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    url: self.resolve_url(url, base_url)?,
                    icons,
                };
                result.push(shortcut);
            }
        }

        Ok(result)
    }

    fn parse_screenshots(&self, screenshots: &[Value], base_url: Option<&str>) -> Result<Vec<super::Screenshot>, ManifestError> {
        let mut result = Vec::new();

        for screenshot_value in screenshots {
            if let Some(src) = screenshot_value.get("src").and_then(|v| v.as_str()) {
                let screenshot = super::Screenshot {
                    src: self.resolve_url(src, base_url)?,
                    sizes: screenshot_value.get("sizes").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    screenshot_type: screenshot_value.get("type").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    form_factor: screenshot_value.get("form_factor")
                        .and_then(|v| v.as_str())
                        .and_then(|s| self.parse_form_factor(s).ok()),
                    label: screenshot_value.get("label").and_then(|v| v.as_str()).map(|s| s.to_string()),
                };
                result.push(screenshot);
            }
        }

        Ok(result)
    }

    fn parse_form_factor(&self, form_factor: &str) -> Result<super::FormFactor, ManifestError> {
        match form_factor {
            "narrow" => Ok(super::FormFactor::Narrow),
            "wide" => Ok(super::FormFactor::Wide),
            _ => Ok(super::FormFactor::Narrow),
        }
    }

    fn parse_related_applications(&self, related_apps: &[Value]) -> Result<Vec<super::RelatedApplication>, ManifestError> {
        let mut result = Vec::new();

        for app_value in related_apps {
            if let Some(platform) = app_value.get("platform").and_then(|v| v.as_str()) {
                let app = super::RelatedApplication {
                    platform: platform.to_string(),
                    url: app_value.get("url").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    id: app_value.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                };
                result.push(app);
            }
        }

        Ok(result)
    }

    fn validate_manifest(&self, manifest: &Manifest) -> Result<(), ManifestError> {
        if manifest.name.is_empty() {
            return Err(ManifestError::InvalidManifest("Name cannot be empty".to_string()));
        }

        if manifest.start_url.is_empty() {
            return Err(ManifestError::InvalidManifest("Start URL cannot be empty".to_string()));
        }

        Ok(())
    }
}

impl Default for ManifestParser {
    fn default() -> Self {
        Self::new()
    }
}