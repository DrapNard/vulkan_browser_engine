pub mod engine;
pub mod flexbox;
pub mod grid;

pub use engine::{
    LayoutBox, LayoutConstraints, LayoutEngine, LayoutError, LayoutMetrics, LayoutResult,
};
pub use flexbox::{
    AlignContent as FlexAlignContent, AlignItems as FlexAlignItems, AlignSelf, FlexContainer,
    FlexDirection, FlexItem, FlexLine, FlexWrap, FlexboxLayout,
    JustifyContent as FlexJustifyContent,
};
pub use grid::{
    AlignContent as GridAlignContent, AlignItems as GridAlignItems, GridArea, GridAutoFlow,
    GridContainer, GridItem, GridLayout, GridLine, JustifyContent as GridJustifyContent,
    JustifyItems, TrackSize,
};

use parking_lot::RwLock;
use std::sync::Arc;
use thiserror::Error;

use crate::core::{
    css::{ComputedValue, StyleEngine},
    dom::{Document, NodeId},
};

#[derive(Error, Debug)]
pub enum LayoutManagerError {
    #[error("Layout engine error: {0}")]
    Engine(#[from] LayoutError),
    #[error("Invalid layout tree: {0}")]
    InvalidTree(String),
    #[error("Layout computation timeout: {0}")]
    Timeout(String),
    #[error("Memory limit exceeded: {0}")]
    MemoryLimit(String),
}

pub type Result<T> = std::result::Result<T, LayoutManagerError>;

#[derive(Debug, Clone)]
pub struct LayoutManagerConfig {
    pub enable_parallel_layout: bool,
    pub parallel_threshold: usize,
    pub max_layout_time_ms: u64,
    pub max_memory_mb: usize,
    pub enable_layout_cache: bool,
    pub cache_size_limit: usize,
}

impl Default for LayoutManagerConfig {
    fn default() -> Self {
        Self {
            enable_parallel_layout: true,
            parallel_threshold: 100,
            max_layout_time_ms: 16, // Target 60 FPS
            max_memory_mb: 512,
            enable_layout_cache: true,
            cache_size_limit: 10000,
        }
    }
}

pub struct LayoutManager {
    engine: Arc<LayoutEngine>,
    config: LayoutManagerConfig,
    performance_monitor: Arc<RwLock<LayoutPerformanceMonitor>>,
}

#[derive(Debug, Clone, Default)]
pub struct LayoutPerformanceMonitor {
    pub total_layouts: u64,
    pub average_layout_time_ms: f64,
    pub max_layout_time_ms: u64,
    pub cache_hit_rate: f64,
    pub memory_usage_mb: f64,
    pub last_layout_timestamp: Option<std::time::Instant>,
}

impl LayoutManager {
    pub fn new(viewport_width: u32, viewport_height: u32, config: LayoutManagerConfig) -> Self {
        let engine = Arc::new(LayoutEngine::new(viewport_width, viewport_height));

        Self {
            engine,
            config,
            performance_monitor: Arc::new(RwLock::new(LayoutPerformanceMonitor::default())),
        }
    }

    pub async fn compute_layout(
        &self,
        document: &Document,
        style_engine: &StyleEngine,
    ) -> Result<()> {
        let start_time = std::time::Instant::now();

        // Check if we need to timeout early
        let timeout_duration = std::time::Duration::from_millis(self.config.max_layout_time_ms);

        let layout_future = self.engine.compute_layout(document, style_engine);

        match tokio::time::timeout(timeout_duration, layout_future).await {
            Ok(result) => {
                result.map_err(LayoutManagerError::from)?;

                start_time.elapsed();
                self.update_performance_metrics().await;

                Ok(())
            }
            Err(_) => Err(LayoutManagerError::Timeout(format!(
                "Layout computation exceeded {}ms timeout",
                self.config.max_layout_time_ms
            ))),
        }
    }

    pub async fn invalidate_node(&self, node_id: NodeId) -> Result<()> {
        self.engine.invalidate_node(node_id).await;
        Ok(())
    }

    pub async fn invalidate_subtree(&self, node_id: NodeId, document: &Document) -> Result<()> {
        self.engine.invalidate_subtree(node_id, document).await;
        Ok(())
    }

    pub async fn resize_viewport(&self, width: u32, height: u32) -> Result<()> {
        self.engine
            .resize_viewport(width, height)
            .await
            .map_err(LayoutManagerError::from)
    }

    pub fn get_layout_box(&self, node_id: NodeId) -> Option<LayoutBox> {
        self.engine.get_layout_box(node_id)
    }

    pub fn get_layout_result(&self, node_id: NodeId) -> Option<LayoutResult> {
        self.engine.get_layout_result(node_id)
    }

    pub async fn get_performance_metrics(&self) -> LayoutPerformanceMonitor {
        let engine_metrics = self.engine.get_metrics().await;
        let mut monitor = self.performance_monitor.write();

        monitor.total_layouts = engine_metrics.total_layouts;
        monitor.average_layout_time_ms = engine_metrics.average_layout_time_us / 1000.0;
        monitor.max_layout_time_ms = (engine_metrics.max_layout_time_us / 1000.0) as u64;
        monitor.cache_hit_rate = if engine_metrics.cache_hits + engine_metrics.cache_misses > 0 {
            engine_metrics.cache_hits as f64
                / (engine_metrics.cache_hits + engine_metrics.cache_misses) as f64
        } else {
            0.0
        };
        monitor.memory_usage_mb = engine_metrics.memory_usage_bytes as f64 / (1024.0 * 1024.0);

        monitor.clone()
    }

    async fn update_performance_metrics(&self) {
        let mut monitor = self.performance_monitor.write();
        monitor.last_layout_timestamp = Some(std::time::Instant::now());

        // Check memory usage
        let engine_stats = self.engine.get_cache_stats();
        if let Some(memory_mb) = engine_stats.get("memory_usage_mb").and_then(|v| v.as_f64()) {
            if memory_mb > self.config.max_memory_mb as f64 {
                tracing::warn!(
                    "Layout memory usage ({:.2} MB) exceeds limit ({} MB)",
                    memory_mb,
                    self.config.max_memory_mb
                );

                // Clear some cache to free memory
                self.engine.clear_cache();
            }
        }
    }

    pub fn get_cache_stats(&self) -> serde_json::Value {
        self.engine.get_cache_stats()
    }

    pub fn clear_cache(&self) {
        self.engine.clear_cache();
    }

    pub fn get_config(&self) -> &LayoutManagerConfig {
        &self.config
    }

    pub fn update_config(&mut self, config: LayoutManagerConfig) {
        self.config = config;
    }
}

// Utility functions for layout calculations
pub mod utils {
    use super::*;

    pub fn calculate_intrinsic_width(node_id: NodeId, style_engine: &StyleEngine) -> f32 {
        // Simplified intrinsic width calculation
        if let Some(computed_styles) = style_engine.get_computed_styles(node_id) {
            if let Ok(width) = computed_styles.get_computed_value("width") {
                if let ComputedValue::Length(v) = width {
                    return v;
                }
            }
        }
        0.0
    }

    pub fn calculate_intrinsic_height(node_id: NodeId, style_engine: &StyleEngine) -> f32 {
        // Simplified intrinsic height calculation
        if let Some(computed_styles) = style_engine.get_computed_styles(node_id) {
            if let Ok(height) = computed_styles.get_computed_value("height") {
                if let ComputedValue::Length(v) = height {
                    return v;
                }
            }
        }
        0.0
    }

    pub fn is_replaced_element(node_id: NodeId, document: &Document) -> bool {
        if let Some(node) = document.get_node(node_id) {
            let node_guard = node.read();
            match node_guard.get_tag_name() {
                "img" | "video" | "audio" | "canvas" | "iframe" | "object" | "embed" => true,
                "input" => {
                    if let Some(input_type) = node_guard.get_attribute("type") {
                        matches!(input_type.as_str(), "image" | "submit" | "reset" | "button")
                    } else {
                        false
                    }
                }
                _ => false,
            }
        } else {
            false
        }
    }

    pub fn get_baseline(layout_box: &LayoutBox, font_size: f32) -> f32 {
        // Simplified baseline calculation
        layout_box.content_y + font_size * 0.8
    }

    pub fn calculate_min_content_width(
        node_id: NodeId,
        document: &Document,
        style_engine: &StyleEngine,
    ) -> f32 {
        // Simplified min-content width calculation
        if let Some(node) = document.get_node(node_id) {
            let node_guard = node.read();
            if (*node_guard).is_text() {
                let text = node_guard.get_text_content();
                let longest_word = text
                    .split_whitespace()
                    .map(|word| word.len())
                    .max()
                    .unwrap_or(0);

                if let Some(computed_styles) = style_engine.get_computed_styles(node_id) {
                    let font_size = match computed_styles.get_computed_value("font_size") {
                        Ok(ComputedValue::Length(v)) => v,
                        _ => 16.0,
                    };
                    return longest_word as f32 * font_size * 0.6; // Approximation
                }
            }
        }
        0.0
    }

    pub fn calculate_max_content_width(
        node_id: NodeId,
        document: &Document,
        style_engine: &StyleEngine,
    ) -> f32 {
        // Simplified max-content width calculation
        if let Some(node) = document.get_node(node_id) {
            let node_guard = node.read();
            if (*node_guard).is_text() {
                let text = node_guard.get_text_content();

                if let Some(computed_styles) = style_engine.get_computed_styles(node_id) {
                    let font_size = match computed_styles.get_computed_value("font_size") {
                        Ok(ComputedValue::Length(v)) => v,
                        _ => 16.0,
                    };
                    return text.len() as f32 * font_size * 0.6; // Approximation
                }
            }
        }
        0.0
    }

    pub fn resolve_percentage(percentage: f32, base: f32, fallback: f32) -> f32 {
        if base.is_finite() && base >= 0.0 {
            base * percentage / 100.0
        } else {
            fallback
        }
    }

    pub fn clamp_size(size: f32, min_size: Option<f32>, max_size: Option<f32>) -> f32 {
        let mut result = size;

        if let Some(min) = min_size {
            result = result.max(min);
        }

        if let Some(max) = max_size {
            result = result.min(max);
        }

        result
    }

    pub fn calculate_available_space(
        container_size: f32,
        margins: (f32, f32),
        borders: (f32, f32),
        padding: (f32, f32),
    ) -> f32 {
        (container_size - margins.0 - margins.1 - borders.0 - borders.1 - padding.0 - padding.1)
            .max(0.0)
    }

    pub fn distribute_space(
        available_space: f32,
        items: &[f32],
        distribution_type: SpaceDistribution,
    ) -> Vec<f32> {
        let mut positions = Vec::with_capacity(items.len());

        if items.is_empty() {
            return positions;
        }

        let total_item_size: f32 = items.iter().sum();
        let free_space = available_space - total_item_size;

        match distribution_type {
            SpaceDistribution::Start => {
                let mut current = 0.0;
                for &item_size in items {
                    positions.push(current);
                    current += item_size;
                }
            }
            SpaceDistribution::End => {
                let mut current = free_space;
                for &item_size in items {
                    positions.push(current);
                    current += item_size;
                }
            }
            SpaceDistribution::Center => {
                let mut current = free_space / 2.0;
                for &item_size in items {
                    positions.push(current);
                    current += item_size;
                }
            }
            SpaceDistribution::SpaceBetween => {
                if items.len() == 1 {
                    positions.push(0.0);
                } else {
                    let gap = free_space / (items.len() - 1) as f32;
                    let mut current = 0.0;
                    for &item_size in items {
                        positions.push(current);
                        current += item_size + gap;
                    }
                }
            }
            SpaceDistribution::SpaceAround => {
                let gap = free_space / items.len() as f32;
                let mut current = gap / 2.0;
                for &item_size in items {
                    positions.push(current);
                    current += item_size + gap;
                }
            }
            SpaceDistribution::SpaceEvenly => {
                let gap = free_space / (items.len() + 1) as f32;
                let mut current = gap;
                for &item_size in items {
                    positions.push(current);
                    current += item_size + gap;
                }
            }
        }

        positions
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum SpaceDistribution {
        Start,
        End,
        Center,
        SpaceBetween,
        SpaceAround,
        SpaceEvenly,
    }
}

// Text measurement utilities
pub mod text {

    #[derive(Debug, Clone)]
    pub struct TextMetrics {
        pub width: f32,
        pub height: f32,
        pub ascent: f32,
        pub descent: f32,
        pub line_height: f32,
        pub line_count: usize,
    }

    pub fn measure_text(
        text: &str,
        font_size: f32,
        line_height: f32,
        max_width: Option<f32>,
    ) -> TextMetrics {
        // Simplified text measurement - in a real implementation, this would use
        // actual font metrics and text shaping

        let char_width = font_size * 0.6; // Average character width approximation
        let ascent = font_size * 0.8;
        let descent = font_size * 0.2;

        if let Some(max_w) = max_width {
            let chars_per_line = (max_w / char_width) as usize;
            if chars_per_line == 0 {
                return TextMetrics {
                    width: 0.0,
                    height: line_height,
                    ascent,
                    descent,
                    line_height,
                    line_count: 1,
                };
            }

            let words: Vec<&str> = text.split_whitespace().collect();
            let mut lines = Vec::new();
            let mut current_line = String::new();

            for word in words {
                if current_line.is_empty() {
                    current_line = word.to_string();
                } else if current_line.len() + 1 + word.len() <= chars_per_line {
                    current_line.push(' ');
                    current_line.push_str(word);
                } else {
                    lines.push(current_line);
                    current_line = word.to_string();
                }
            }

            if !current_line.is_empty() {
                lines.push(current_line);
            }

            let line_count = lines.len().max(1);
            let width = lines
                .iter()
                .map(|line| line.len() as f32 * char_width)
                .fold(0.0f32, f32::max)
                .min(max_w);
            let height = line_count as f32 * line_height;

            TextMetrics {
                width,
                height,
                ascent,
                descent,
                line_height,
                line_count,
            }
        } else {
            let width = text.len() as f32 * char_width;
            let height = line_height;

            TextMetrics {
                width,
                height,
                ascent,
                descent,
                line_height,
                line_count: 1,
            }
        }
    }

    pub fn break_text_into_lines(text: &str, max_width: f32, font_size: f32) -> Vec<String> {
        let char_width = font_size * 0.6;
        let chars_per_line = (max_width / char_width) as usize;

        if chars_per_line == 0 {
            return vec![text.to_string()];
        }

        let words: Vec<&str> = text.split_whitespace().collect();
        let mut lines = Vec::new();
        let mut current_line = String::new();

        for word in words {
            if current_line.is_empty() {
                current_line = word.to_string();
            } else if current_line.len() + 1 + word.len() <= chars_per_line {
                current_line.push(' ');
                current_line.push_str(word);
            } else {
                lines.push(current_line);
                current_line = word.to_string();
            }
        }

        if !current_line.is_empty() {
            lines.push(current_line);
        }

        if lines.is_empty() {
            lines.push(String::new());
        }

        lines
    }
}

// Layout debugging utilities
#[cfg(debug_assertions)]
pub mod debug {
    use super::*;

    pub fn print_layout_tree(
        node_id: NodeId,
        document: &Document,
        layout_manager: &LayoutManager,
        indent: usize,
    ) {
        let indent_str = "  ".repeat(indent);

        if let Some(layout_box) = layout_manager.get_layout_box(node_id) {
            if let Some(node) = document.get_node(node_id) {
                let node_guard = node.read();
                println!(
                    "{}Node: {} ({}, {}) {}x{}",
                    indent_str,
                    node_guard.get_tag_name(),
                    layout_box.content_x,
                    layout_box.content_y,
                    layout_box.content_width,
                    layout_box.content_height
                );
            }
        }

        let children = document.get_children(node_id);
        for child_id in children {
            print_layout_tree(child_id, document, layout_manager, indent + 1);
        }
    }

    pub fn validate_layout_tree(
        node_id: NodeId,
        document: &Document,
        layout_manager: &LayoutManager,
    ) -> Vec<String> {
        let mut errors = Vec::new();

        if let Some(layout_box) = layout_manager.get_layout_box(node_id) {
            // Check for negative dimensions
            if layout_box.content_width < 0.0 {
                errors.push(format!(
                    "Node {:?} has negative width: {}",
                    node_id, layout_box.content_width
                ));
            }

            if layout_box.content_height < 0.0 {
                errors.push(format!(
                    "Node {:?} has negative height: {}",
                    node_id, layout_box.content_height
                ));
            }

            // Check for infinite or NaN values
            if !layout_box.content_x.is_finite() {
                errors.push(format!(
                    "Node {:?} has invalid x position: {}",
                    node_id, layout_box.content_x
                ));
            }

            if !layout_box.content_y.is_finite() {
                errors.push(format!(
                    "Node {:?} has invalid y position: {}",
                    node_id, layout_box.content_y
                ));
            }
        }

        let children = document.get_children(node_id);
        for child_id in children {
            errors.extend(validate_layout_tree(child_id, document, layout_manager));
        }

        errors
    }
}
