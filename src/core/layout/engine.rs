use std::sync::Arc;
use parking_lot::{RwLock};
use dashmap::DashMap;
use smallvec::SmallVec;
use thiserror::Error;
use rayon::prelude::*;
use async_recursion::async_recursion;


use super::{flexbox::FlexboxLayout, grid::GridLayout};
use crate::core::{
    dom::{Document, NodeId, Node, DisplayType},
    css::{ComputedStyles, StyleEngine, ComputedValue},
};

#[derive(Error, Debug)]
pub enum LayoutError {
    #[error("Layout computation failed: {0}")]
    Computation(String),
    #[error("Constraint resolution failed: {0}")]
    ConstraintResolution(String),
    #[error("Invalid layout tree: {0}")]
    InvalidTree(String),
    #[error("Circular dependency detected: {0}")]
    CircularDependency(String),
    #[error("Memory allocation failed: {0}")]
    Memory(String),
}

pub type Result<T> = std::result::Result<T, LayoutError>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutBox {
    pub content_x: f32,
    pub content_y: f32,
    pub content_width: f32,
    pub content_height: f32,
    pub padding_top: f32,
    pub padding_right: f32,
    pub padding_bottom: f32,
    pub padding_left: f32,
    pub border_top: f32,
    pub border_right: f32,
    pub border_bottom: f32,
    pub border_left: f32,
    pub margin_top: f32,
    pub margin_right: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
}

impl Default for LayoutBox {
    fn default() -> Self {
        Self {
            content_x: 0.0,
            content_y: 0.0,
            content_width: 0.0,
            content_height: 0.0,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            border_top: 0.0,
            border_right: 0.0,
            border_bottom: 0.0,
            border_left: 0.0,
            margin_top: 0.0,
            margin_right: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
        }
    }
}

impl LayoutBox {
    pub fn padding_box_x(&self) -> f32 {
        self.content_x - self.padding_left
    }

    pub fn padding_box_y(&self) -> f32 {
        self.content_y - self.padding_top
    }

    pub fn padding_box_width(&self) -> f32 {
        self.content_width + self.padding_left + self.padding_right
    }

    pub fn padding_box_height(&self) -> f32 {
        self.content_height + self.padding_top + self.padding_bottom
    }

    pub fn border_box_x(&self) -> f32 {
        self.padding_box_x() - self.border_left
    }

    pub fn border_box_y(&self) -> f32 {
        self.padding_box_y() - self.border_top
    }

    pub fn border_box_width(&self) -> f32 {
        self.padding_box_width() + self.border_left + self.border_right
    }

    pub fn border_box_height(&self) -> f32 {
        self.padding_box_height() + self.border_top + self.border_bottom
    }

    pub fn margin_box_x(&self) -> f32 {
        self.border_box_x() - self.margin_left
    }

    pub fn margin_box_y(&self) -> f32 {
        self.border_box_y() - self.margin_top
    }

    pub fn margin_box_width(&self) -> f32 {
        self.border_box_width() + self.margin_left + self.margin_right
    }

    pub fn margin_box_height(&self) -> f32 {
        self.border_box_height() + self.margin_top + self.margin_bottom
    }

    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        let bx = self.border_box_x();
        let by = self.border_box_y();
        let bw = self.border_box_width();
        let bh = self.border_box_height();
        
        x >= bx && x <= bx + bw && y >= by && y <= by + bh
    }

    pub fn intersects(&self, other: &LayoutBox) -> bool {
        let x1 = self.border_box_x();
        let y1 = self.border_box_y();
        let w1 = self.border_box_width();
        let h1 = self.border_box_height();
        
        let x2 = other.border_box_x();
        let y2 = other.border_box_y();
        let w2 = other.border_box_width();
        let h2 = other.border_box_height();
        
        !(x1 + w1 < x2 || x2 + w2 < x1 || y1 + h1 < y2 || y2 + h2 < y1)
    }
}

#[derive(Debug, Clone)]
pub struct LayoutConstraints {
    pub available_width: Option<f32>,
    pub available_height: Option<f32>,
    pub min_width: f32,
    pub max_width: Option<f32>,
    pub min_height: f32,
    pub max_height: Option<f32>,
    pub baseline: Option<f32>,
}

impl Default for LayoutConstraints {
    fn default() -> Self {
        Self {
            available_width: None,
            available_height: None,
            min_width: 0.0,
            max_width: None,
            min_height: 0.0,
            max_height: None,
            baseline: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LayoutResult {
    pub layout_box: LayoutBox,
    pub baseline: Option<f32>,
    pub intrinsic_width: f32,
    pub intrinsic_height: f32,
    pub children_overflow: bool,
}

impl Default for LayoutResult {
    fn default() -> Self {
        Self {
            layout_box: LayoutBox::default(),
            baseline: None,
            intrinsic_width: 0.0,
            intrinsic_height: 0.0,
            children_overflow: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LayoutCache {
    constraints: LayoutConstraints,
    result: LayoutResult,
    generation: u64,
    dependencies: SmallVec<[NodeId; 4]>,
}

pub struct LayoutEngine {
    viewport_width: Arc<RwLock<f32>>,
    viewport_height: Arc<RwLock<f32>>,
    layout_cache: Arc<DashMap<NodeId, LayoutCache>>,
    invalidation_queue: Arc<RwLock<Vec<NodeId>>>,
    layout_generation: Arc<RwLock<u64>>,
    flexbox_layout: Arc<FlexboxLayout>,
    grid_layout: Arc<GridLayout>,
    parallel_threshold: usize,
    performance_metrics: Arc<RwLock<LayoutMetrics>>,
}

#[derive(Debug, Clone, Default)]
pub struct LayoutMetrics {
    pub total_layouts: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub parallel_layouts: u64,
    pub average_layout_time_us: f64,
    pub max_layout_time_us: f64,
    pub memory_usage_bytes: usize,
}

impl LayoutEngine {
    pub fn new(viewport_width: u32, viewport_height: u32) -> Self {
        Self {
            viewport_width: Arc::new(RwLock::new(viewport_width as f32)),
            viewport_height: Arc::new(RwLock::new(viewport_height as f32)),
            layout_cache: Arc::new(DashMap::new()),
            invalidation_queue: Arc::new(RwLock::new(Vec::new())),
            layout_generation: Arc::new(RwLock::new(0)),
            flexbox_layout: Arc::new(FlexboxLayout::new()),
            grid_layout: Arc::new(GridLayout::new()),
            parallel_threshold: 100, // Use parallel layout for nodes with 100+ children
            performance_metrics: Arc::new(RwLock::new(LayoutMetrics::default())),
        }
    }

    pub async fn compute_layout(
        &self,
        document: &Document,
        style_engine: &StyleEngine,
    ) -> Result<()> {
        let start_time = std::time::Instant::now();
        
        self.process_invalidation_queue().await;
        
        let mut generation = self.layout_generation.write();
        *generation += 1;
        let current_generation = *generation;
        drop(generation);

        if let Some(root_id) = document.get_root_node() {
            let viewport_width = *self.viewport_width.read();
            let viewport_height = *self.viewport_height.read();
            
            let constraints = LayoutConstraints {
                available_width: Some(viewport_width),
                available_height: Some(viewport_height),
                ..Default::default()
            };

            self.layout_node_recursive(
                root_id,
                constraints,
                document,
                style_engine,
                current_generation,
            ).await?;
        }

        let layout_time = start_time.elapsed();
        self.update_performance_metrics(layout_time).await;

        Ok(())
    }

    #[async_recursion(?Send)]
    async fn layout_node_recursive(
        &self,
        node_id: NodeId,
        constraints: LayoutConstraints,
        document: &Document,
        style_engine: &StyleEngine,
        generation: u64,
    ) -> Result<LayoutResult> {
        if let Some(cached) = self.get_cached_layout(node_id, &constraints, generation) {
            let mut metrics = self.performance_metrics.write();
            metrics.cache_hits += 1;
            return Ok(cached.result);
        }

        let mut metrics = self.performance_metrics.write();
        metrics.cache_misses += 1;
        metrics.total_layouts += 1;
        drop(metrics);

        let computed_styles = style_engine.get_computed_styles(node_id)
            .ok_or_else(|| LayoutError::Computation("No computed styles found".to_string()))?;

        let display = self.get_display_type(&computed_styles)?;
        
        let result = match display {
            DisplayType::None => {
                LayoutResult::default()
            }
            DisplayType::Block => {
                self.layout_block_node(node_id, constraints, document, style_engine, generation).await?
            }
            DisplayType::Inline => {
                self.layout_inline_node(node_id, constraints, document, style_engine, generation).await?
            }
            DisplayType::InlineBlock => {
                self.layout_inline_block_node(node_id, constraints, document, style_engine, generation).await?
            }
            DisplayType::Flex => {
                self.flexbox_layout.layout_flex_container(
                    node_id,
                    constraints,
                    document,
                    style_engine,
                    generation,
                    self,
                ).await?
            }
            DisplayType::Grid => {
                self.grid_layout.layout_grid_container(
                    node_id,
                    constraints,
                    document,
                    style_engine,
                    generation,
                    self,
                ).await?
            }
            _ => {
                self.layout_block_node(node_id, constraints, document, style_engine, generation).await?
            }
        };

        self.cache_layout_result(node_id, constraints.clone(), result.clone(), generation);

        Ok(result)
    }

    async fn layout_block_node(
        &self,
        node_id: NodeId,
        constraints: LayoutConstraints,
        document: &Document,
        style_engine: &StyleEngine,
        generation: u64,
    ) -> Result<LayoutResult> {
        let computed_styles = style_engine.get_computed_styles(node_id).unwrap();
        
        let mut layout_box = self.compute_box_model(&computed_styles, &constraints)?;
        
        let content_constraints = LayoutConstraints {
            available_width: Some(layout_box.content_width),
            available_height: constraints.available_height,
            ..Default::default()
        };

        let children = document.get_children(node_id);
        
        if children.len() > self.parallel_threshold {
            self.layout_children_parallel(
                &children,
                content_constraints,
                document,
                style_engine,
                generation,
            ).await?;
        } else {
            self.layout_children_sequential(
                &children,
                content_constraints,
                document,
                style_engine,
                generation,
            ).await?;
        }

        let mut current_y = layout_box.content_y;
        let mut max_width = 0.0f32;
        let mut children_overflow = false;

        for &child_id in &children {
            if let Some(child_cache) = self.layout_cache.get(&child_id) {
                let child_box = &child_cache.result.layout_box;
                
                layout_box.content_y = current_y;
                current_y += child_box.margin_box_height();
                max_width = max_width.max(child_box.margin_box_width());
                
                if child_cache.result.children_overflow {
                    children_overflow = true;
                }
            }
        }

        if constraints.available_height.is_none() {
            layout_box.content_height = current_y - layout_box.content_y;
        }

        if children_overflow || current_y > layout_box.content_y + layout_box.content_height {
            children_overflow = true;
        }

        Ok(LayoutResult {
            layout_box,
            baseline: Some(layout_box.content_y + layout_box.content_height),
            intrinsic_width: max_width,
            intrinsic_height: current_y - layout_box.content_y,
            children_overflow,
        })
    }

    async fn layout_inline_node(
        &self,
        _node_id: NodeId,
        constraints: LayoutConstraints,
        _document: &Document,
        style_engine: &StyleEngine,
        _generation: u64,
    ) -> Result<LayoutResult> {
        let computed_styles = style_engine.get_computed_styles(_node_id).unwrap();
        let layout_box = self.compute_box_model(&computed_styles, &constraints)?;
        Ok(LayoutResult {
            layout_box,
            baseline: Some(layout_box.content_y + layout_box.content_height * 0.8),
            intrinsic_width: layout_box.content_width,
            intrinsic_height: layout_box.content_height,
            children_overflow: false,
        })
    }

    async fn layout_inline_block_node(
        &self,
        node_id: NodeId,
        constraints: LayoutConstraints,
        document: &Document,
        style_engine: &StyleEngine,
        generation: u64,
    ) -> Result<LayoutResult> {
        self.layout_block_node(node_id, constraints, document, style_engine, generation).await
    }

    async fn layout_text_node(
        &self,
        node: &Node,
        computed_styles: &ComputedStyles,
        constraints: &LayoutConstraints,
    ) -> Result<LayoutResult> {
        let text_content = node.get_text_content();
        let font_size = match computed_styles.get_computed_value("font-size") {
            Ok(ComputedValue::Length(length)) => length,
            Ok(ComputedValue::Percentage(percentage)) => constraints.available_width.map(|w| w * percentage / 100.0).unwrap_or(16.0),
            _ => 16.0, // Default font size
        };
        let line_height = match computed_styles.get_computed_value("line-height") {
            Ok(ComputedValue::Length(length)) => length,
            Ok(ComputedValue::Percentage(percentage)) => font_size * percentage / 100.0,
            _ => font_size * 1.2, // Default line height
        };
        
        let available_width = constraints.available_width.unwrap_or(f32::INFINITY);
        
        let (text_width, text_height, line_count) = self.measure_text(
            &text_content,
            font_size,
            line_height,
            available_width,
        );

        let layout_box = LayoutBox {
            content_x: 0.0,
            content_y: 0.0,
            content_width: text_width,
            content_height: text_height,
            ..Default::default()
        };

        Ok(LayoutResult {
            layout_box,
            baseline: Some(line_height * 0.8),
            intrinsic_width: text_width,
            intrinsic_height: text_height,
            children_overflow: line_count > 1 && text_height > constraints.available_height.unwrap_or(f32::INFINITY),
        })
    }

    fn measure_text(
        &self,
        text: &str,
        font_size: f32,
        line_height: f32,
        available_width: f32,
    ) -> (f32, f32, usize) {
        let char_width = font_size * 0.6; // Approximation
        let chars_per_line = (available_width / char_width) as usize;
        
        if chars_per_line == 0 || text.is_empty() {
            return (0.0, 0.0, 0);
        }

        let words: Vec<&str> = text.split_whitespace().collect();
        let mut lines = Vec::new();
        let mut current_line = String::new();
        let mut current_width = 0.0;

        for word in words {
            let word_width = word.len() as f32 * char_width;
            let space_width = char_width;

            if current_line.is_empty() {
                current_line = word.to_string();
                current_width = word_width;
            } else if current_width + space_width + word_width <= available_width {
                current_line.push(' ');
                current_line.push_str(word);
                current_width += space_width + word_width;
            } else {
                lines.push(current_line);
                current_line = word.to_string();
                current_width = word_width;
            }
        }

        if !current_line.is_empty() {
            lines.push(current_line);
        }

        let line_count = lines.len().max(1);
        let text_width = if available_width == f32::INFINITY {
            lines.iter()
                .map(|line| line.len() as f32 * char_width)
                .fold(0.0f32, f32::max)
        } else {
            available_width.min(text.len() as f32 * char_width)
        };

        let text_height = line_count as f32 * line_height;

        (text_width, text_height, line_count)
    }

    async fn layout_children_parallel(
        &self,
        children: &[NodeId],
        constraints: LayoutConstraints,
        document: &Document,
        style_engine: &StyleEngine,
        generation: u64,
    ) -> Result<()> {
        let results: Vec<Result<LayoutResult>> = children
            .par_iter()
            .map(|&child_id| {
                futures::executor::block_on(self.layout_node_recursive(
                    child_id,
                    constraints.clone(),
                    document,
                    style_engine,
                    generation,
                ))
            })
            .collect();

        for result in results {
            result?;
        }

        let mut metrics = self.performance_metrics.write();
        metrics.parallel_layouts += 1;

        Ok(())
    }

    async fn layout_children_sequential(
        &self,
        children: &[NodeId],
        constraints: LayoutConstraints,
        document: &Document,
        style_engine: &StyleEngine,
        generation: u64,
    ) -> Result<()> {
        for &child_id in children {
            self.layout_node_recursive(
                child_id,
                constraints.clone(),
                document,
                style_engine,
                generation,
            ).await?;
        }

        Ok(())
    }

    fn compute_box_model(
        &self,
        computed_styles: &ComputedStyles,
        constraints: &LayoutConstraints,
    ) -> Result<LayoutBox> {
        let width = self.resolve_length_property(computed_styles, "width", constraints.available_width)?;
        let height = self.resolve_length_property(computed_styles, "height", constraints.available_height)?;

        let pt = match computed_styles.get_computed_value("padding-top") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let pr = match computed_styles.get_computed_value("padding-right") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let pb = match computed_styles.get_computed_value("padding-bottom") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let pl = match computed_styles.get_computed_value("padding-left") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let bt = match computed_styles.get_computed_value("border-top-width") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let br = match computed_styles.get_computed_value("border-right-width") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let bb = match computed_styles.get_computed_value("border-bottom-width") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let bl = match computed_styles.get_computed_value("border-left-width") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let mt = match computed_styles.get_computed_value("margin-top") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let mr = match computed_styles.get_computed_value("margin-right") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let mb = match computed_styles.get_computed_value("margin-bottom") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let ml = match computed_styles.get_computed_value("margin-left") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };

        let content_width = width
            .or(constraints.available_width.map(|av| av - pl - pr - bl - br - ml - mr))
            .unwrap_or(0.0)
            .max(constraints.min_width);

        let content_height = height.unwrap_or(0.0).max(constraints.min_height);

        Ok(LayoutBox {
            content_x: ml + bl + pl,
            content_y: mt + bt + pt,
            content_width,
            content_height,
            padding_top: pt,
            padding_right: pr,
            padding_bottom: pb,
            padding_left: pl,
            border_top: bt,
            border_right: br,
            border_bottom: bb,
            border_left: bl,
            margin_top: mt,
            margin_right: mr,
            margin_bottom: mb,
            margin_left: ml,
        })

    }

    fn resolve_length_property(
        &self,
        computed_styles: &ComputedStyles,
        property: &str,
        available: Option<f32>,
    ) -> Result<Option<f32>> {
        match computed_styles.get_computed_value(property) {
            Ok(value) => match value {
                crate::core::css::ComputedValue::Length(length) => Ok(Some(length)),
                crate::core::css::ComputedValue::Percentage(percentage) => {
                    if let Some(available_size) = available {
                        Ok(Some(available_size * percentage / 100.0))
                    } else {
                        Ok(None)
                    }
                }
                crate::core::css::ComputedValue::Auto => Ok(None),
                _ => Ok(None),
            },
            Err(_) => Ok(None),
        }
    }

    fn get_display_type(&self, computed_styles: &ComputedStyles) -> Result<DisplayType> {
        match computed_styles.get_computed_value("display") {
            Ok(value) => match value {
                crate::core::css::ComputedValue::Keyword(keyword) => {
                    match keyword.as_str() {
                        "none" => Ok(DisplayType::None),
                        "block" => Ok(DisplayType::Block),
                        "inline" => Ok(DisplayType::Inline),
                        "inline-block" => Ok(DisplayType::InlineBlock),
                        "flex" => Ok(DisplayType::Flex),
                        "grid" => Ok(DisplayType::Grid),
                        "table" => Ok(DisplayType::Table),
                        "table-row" => Ok(DisplayType::TableRow),
                        "table-cell" => Ok(DisplayType::TableCell),
                        "list-item" => Ok(DisplayType::ListItem),
                        _ => Ok(DisplayType::Block),
                    }
                }
                _ => Ok(DisplayType::Block),
            },
            Err(_) => Ok(DisplayType::Block),
        }
    }

    fn get_cached_layout(
        &self,
        node_id: NodeId,
        constraints: &LayoutConstraints,
        generation: u64,
    ) -> Option<LayoutCache> {
        if let Some(cached) = self.layout_cache.get(&node_id) {
            if cached.generation == generation && self.constraints_match(&cached.constraints, constraints) {
                return Some(cached.clone());
            }
        }
        None
    }

    fn constraints_match(&self, cached: &LayoutConstraints, current: &LayoutConstraints) -> bool {
        const EPSILON: f32 = 0.001;
        
        self.option_f32_eq(cached.available_width, current.available_width, EPSILON) &&
        self.option_f32_eq(cached.available_height, current.available_height, EPSILON) &&
        (cached.min_width - current.min_width).abs() < EPSILON &&
        self.option_f32_eq(cached.max_width, current.max_width, EPSILON) &&
        (cached.min_height - current.min_height).abs() < EPSILON &&
        self.option_f32_eq(cached.max_height, current.max_height, EPSILON)
    }

    fn option_f32_eq(&self, a: Option<f32>, b: Option<f32>, epsilon: f32) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => (a - b).abs() < epsilon,
            (None, None) => true,
            _ => false,
        }
    }

    fn cache_layout_result(
        &self,
        node_id: NodeId,
        constraints: LayoutConstraints,
        result: LayoutResult,
        generation: u64,
    ) {
        let cache_entry = LayoutCache {
            constraints,
            result,
            generation,
            dependencies: SmallVec::new(),
        };

        self.layout_cache.insert(node_id, cache_entry);
    }

    async fn process_invalidation_queue(&self) {
        let mut queue = self.invalidation_queue.write();
        for node_id in queue.drain(..) {
            self.layout_cache.remove(&node_id);
        }
    }

    pub async fn invalidate_node(&self, node_id: NodeId) {
        self.invalidation_queue.write().push(node_id);
    }

    pub async fn invalidate_subtree(&self, node_id: NodeId, document: &Document) {
        let mut to_invalidate = vec![node_id];
        let mut visited = std::collections::HashSet::new();

        while let Some(current_id) = to_invalidate.pop() {
            if visited.insert(current_id) {
                self.invalidate_node(current_id).await;
                
                let children = document.get_children(current_id);
                to_invalidate.extend(children);
            }
        }
    }

    pub async fn resize_viewport(&self, width: u32, height: u32) -> Result<()> {
        {
            let mut vw = self.viewport_width.write();
            let mut vh = self.viewport_height.write();
            *vw = width as f32;
            *vh = height as f32;
        }

        self.layout_cache.clear();

        Ok(())
    }

    pub fn get_layout_box(&self, node_id: NodeId) -> Option<LayoutBox> {
        self.layout_cache.get(&node_id).map(|cache| cache.result.layout_box)
    }

    pub fn get_layout_result(&self, node_id: NodeId) -> Option<LayoutResult> {
        self.layout_cache.get(&node_id).map(|cache| cache.result.clone())
    }

    async fn update_performance_metrics(&self, layout_time: std::time::Duration) {
        let mut metrics = self.performance_metrics.write();
        
        let layout_time_us = layout_time.as_micros() as f64;
        
        if metrics.total_layouts == 1 {
            metrics.average_layout_time_us = layout_time_us;
            metrics.max_layout_time_us = layout_time_us;
        } else {
            let alpha = 0.1; // Exponential moving average factor
            metrics.average_layout_time_us = 
                alpha * layout_time_us + (1.0 - alpha) * metrics.average_layout_time_us;
            metrics.max_layout_time_us = metrics.max_layout_time_us.max(layout_time_us);
        }

        metrics.memory_usage_bytes = self.layout_cache.len() * std::mem::size_of::<LayoutCache>();
    }

    pub async fn get_metrics(&self) -> LayoutMetrics {
        self.performance_metrics.read().clone()
    }

    pub fn clear_cache(&self) {
        self.layout_cache.clear();
        let mut metrics = self.performance_metrics.write();
        *metrics = LayoutMetrics::default();
    }

    pub fn get_cache_stats(&self) -> serde_json::Value {
        let metrics = self.performance_metrics.read();
        
        serde_json::json!({
            "cache_size": self.layout_cache.len(),
            "total_layouts": metrics.total_layouts,
            "cache_hits": metrics.cache_hits,
            "cache_misses": metrics.cache_misses,
            "cache_hit_ratio": if metrics.cache_hits + metrics.cache_misses > 0 {
                metrics.cache_hits as f64 / (metrics.cache_hits + metrics.cache_misses) as f64
            } else { 0.0 },
            "parallel_layouts": metrics.parallel_layouts,
            "average_layout_time_us": metrics.average_layout_time_us,
            "max_layout_time_us": metrics.max_layout_time_us,
            "memory_usage_mb": metrics.memory_usage_bytes as f64 / (1024.0 * 1024.0),
        })
    }

    pub async fn layout_node_public(
        &self,
        node_id: NodeId,
        constraints: LayoutConstraints,
        document: &Document,
        style_engine: &StyleEngine,
        generation: u64,
    ) -> Result<LayoutResult> {
        self.layout_node_recursive(node_id, constraints, document, style_engine, generation).await
    }
}