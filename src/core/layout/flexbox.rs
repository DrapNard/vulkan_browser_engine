use std::sync::Arc;
use thiserror::Error;

use super::engine::{LayoutBox, LayoutConstraints, LayoutEngine, LayoutError, LayoutResult};
use crate::core::{
    css::{ComputedStyles, ComputedValue, StyleEngine},
    dom::{Document, NodeId},
};

#[derive(Error, Debug)]
pub enum FlexboxError {
    #[error("Flexbox computation failed: {0}")]
    Computation(String),
    #[error("Invalid flex value: {0}")]
    InvalidFlexValue(String),
    #[error("Flex item sizing failed: {0}")]
    ItemSizing(String),
}

pub type Result<T> = std::result::Result<T, FlexboxError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlexDirection {
    #[default]
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlexWrap {
    #[default]
    NoWrap,
    Wrap,
    WrapReverse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JustifyContent {
    #[default]
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignItems {
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
    #[default]
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignContent {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    #[default]
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignSelf {
    #[default]
    Auto,
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
    Stretch,
}

#[derive(Debug, Clone)]
pub struct FlexContainer {
    pub direction: FlexDirection,
    pub wrap: FlexWrap,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub align_content: AlignContent,
    pub gap: f32,
    pub row_gap: f32,
    pub column_gap: f32,
}

impl Default for FlexContainer {
    fn default() -> Self {
        Self {
            direction: FlexDirection::default(),
            wrap: FlexWrap::default(),
            justify_content: JustifyContent::default(),
            align_items: AlignItems::default(),
            align_content: AlignContent::default(),
            gap: 0.0,
            row_gap: 0.0,
            column_gap: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FlexItem {
    pub grow: f32,
    pub shrink: f32,
    pub basis: Option<f32>,
    pub align_self: AlignSelf,
    pub order: i32,
    pub node_id: NodeId,
    pub layout_result: Option<LayoutResult>,
    pub main_size: f32,
    pub cross_size: f32,
    pub target_main_size: f32,
    pub outer_target_main_size: f32,
    pub hypothetical_main_size: f32,
    pub hypothetical_cross_size: f32,
    pub flex_base_size: f32,
    pub scaled_flex_shrink_factor: f32,
    pub is_frozen: bool,
    pub violation: f32,
}

impl FlexItem {
    pub fn new(node_id: NodeId) -> Self {
        Self {
            grow: 0.0,
            shrink: 1.0,
            basis: None,
            align_self: AlignSelf::Auto,
            order: 0,
            node_id,
            layout_result: None,
            main_size: 0.0,
            cross_size: 0.0,
            target_main_size: 0.0,
            outer_target_main_size: 0.0,
            hypothetical_main_size: 0.0,
            hypothetical_cross_size: 0.0,
            flex_base_size: 0.0,
            scaled_flex_shrink_factor: 0.0,
            is_frozen: false,
            violation: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FlexLine {
    pub items: Vec<FlexItem>,
    pub main_size: f32,
    pub cross_size: f32,
    pub baseline: f32,
}

impl Default for FlexLine {
    fn default() -> Self {
        Self::new()
    }
}

impl FlexLine {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            main_size: 0.0,
            cross_size: 0.0,
            baseline: 0.0,
        }
    }
}

pub struct FlexboxLayout {
    cache: Arc<dashmap::DashMap<NodeId, FlexContainer>>,
}

impl Default for FlexboxLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl FlexboxLayout {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(dashmap::DashMap::new()),
        }
    }

    pub async fn layout_flex_container(
        &self,
        node_id: NodeId,
        constraints: LayoutConstraints,
        document: &Document,
        style_engine: &StyleEngine,
        generation: u64,
        layout_engine: &LayoutEngine,
    ) -> std::result::Result<LayoutResult, LayoutError> {
        let computed_styles = style_engine
            .get_computed_styles(node_id)
            .ok_or_else(|| LayoutError::Computation("No computed styles found".to_string()))?;

        let flex_container = self.parse_flex_container(&computed_styles)?;
        self.cache.insert(node_id, flex_container.clone());

        let children = document.get_children(node_id);
        let mut flex_items = self.create_flex_items(&children, style_engine).await?;

        let container_main_size = self.get_main_axis_size(&flex_container, &constraints);
        let container_cross_size = self.get_cross_axis_size(&flex_container, &constraints);

        self.resolve_flex_base_sizes(
            &mut flex_items,
            &flex_container,
            document,
            style_engine,
            layout_engine,
            generation,
        )
        .await?;

        let mut flex_lines =
            self.collect_flex_lines(&mut flex_items, &flex_container, container_main_size);

        self.resolve_flex_lengths(&mut flex_lines, &flex_container, container_main_size);

        self.determine_cross_axis_sizes(
            &mut flex_lines,
            &flex_container,
            document,
            style_engine,
            layout_engine,
            generation,
        )
        .await?;

        self.handle_align_content(&mut flex_lines, &flex_container, container_cross_size);

        let mut layout_box = self.compute_container_box(&computed_styles, &constraints)?;
        self.position_flex_items(&mut flex_lines, &flex_container, &layout_box);

        let total_cross_size = flex_lines.iter().map(|line| line.cross_size).sum::<f32>();
        let max_main_size = flex_lines
            .iter()
            .map(|line| line.main_size)
            .fold(0.0f32, f32::max);

        if constraints.available_height.is_none() {
            layout_box.content_height = total_cross_size;
        }

        let children_overflow = max_main_size > layout_box.content_width
            || total_cross_size > layout_box.content_height;

        Ok(LayoutResult {
            layout_box,
            baseline: flex_lines.first().map(|line| line.baseline),
            intrinsic_width: max_main_size,
            intrinsic_height: total_cross_size,
            children_overflow,
        })
    }

    fn parse_flex_container(
        &self,
        styles: &ComputedStyles,
    ) -> std::result::Result<FlexContainer, LayoutError> {
        let direction = self.parse_flex_direction(styles)?;
        let wrap = self.parse_flex_wrap(styles)?;
        let justify_content = self.parse_justify_content(styles)?;
        let align_items = self.parse_align_items(styles)?;
        let align_content = self.parse_align_content(styles)?;

        let gap = match styles.get_computed_value("gap") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let row_gap = match styles.get_computed_value("row_gap") {
            Ok(ComputedValue::Length(v)) => v,
            _ => gap,
        };
        let column_gap = match styles.get_computed_value("column-gap") {
            Ok(ComputedValue::Length(v)) => v,
            _ => gap,
        };

        Ok(FlexContainer {
            direction,
            wrap,
            justify_content,
            align_items,
            align_content,
            gap,
            row_gap,
            column_gap,
        })
    }

    fn parse_flex_direction(
        &self,
        styles: &ComputedStyles,
    ) -> std::result::Result<FlexDirection, LayoutError> {
        match styles.get_computed_value("flex-direction") {
            Ok(ComputedValue::Keyword(keyword)) => match keyword.as_str() {
                "row" => Ok(FlexDirection::Row),
                "row-reverse" => Ok(FlexDirection::RowReverse),
                "column" => Ok(FlexDirection::Column),
                "column-reverse" => Ok(FlexDirection::ColumnReverse),
                _ => Ok(FlexDirection::Row),
            },
            _ => Ok(FlexDirection::Row),
        }
    }

    fn parse_flex_wrap(
        &self,
        styles: &ComputedStyles,
    ) -> std::result::Result<FlexWrap, LayoutError> {
        match styles.get_computed_value("flex-wrap") {
            Ok(ComputedValue::Keyword(keyword)) => match keyword.as_str() {
                "nowrap" => Ok(FlexWrap::NoWrap),
                "wrap" => Ok(FlexWrap::Wrap),
                "wrap-reverse" => Ok(FlexWrap::WrapReverse),
                _ => Ok(FlexWrap::NoWrap),
            },
            _ => Ok(FlexWrap::NoWrap),
        }
    }

    fn parse_justify_content(
        &self,
        styles: &ComputedStyles,
    ) -> std::result::Result<JustifyContent, LayoutError> {
        match styles.get_computed_value("justify-content") {
            Ok(ComputedValue::Keyword(keyword)) => match keyword.as_str() {
                "flex-start" => Ok(JustifyContent::FlexStart),
                "flex-end" => Ok(JustifyContent::FlexEnd),
                "center" => Ok(JustifyContent::Center),
                "space-between" => Ok(JustifyContent::SpaceBetween),
                "space-around" => Ok(JustifyContent::SpaceAround),
                "space-evenly" => Ok(JustifyContent::SpaceEvenly),
                _ => Ok(JustifyContent::FlexStart),
            },
            _ => Ok(JustifyContent::FlexStart),
        }
    }

    fn parse_align_items(
        &self,
        styles: &ComputedStyles,
    ) -> std::result::Result<AlignItems, LayoutError> {
        match styles.get_computed_value("align-items") {
            Ok(ComputedValue::Keyword(keyword)) => match keyword.as_str() {
                "flex-start" => Ok(AlignItems::FlexStart),
                "flex-end" => Ok(AlignItems::FlexEnd),
                "center" => Ok(AlignItems::Center),
                "baseline" => Ok(AlignItems::Baseline),
                "stretch" => Ok(AlignItems::Stretch),
                _ => Ok(AlignItems::Stretch),
            },
            _ => Ok(AlignItems::Stretch),
        }
    }

    fn parse_align_content(
        &self,
        styles: &ComputedStyles,
    ) -> std::result::Result<AlignContent, LayoutError> {
        match styles.get_computed_value("align-content") {
            Ok(ComputedValue::Keyword(keyword)) => match keyword.as_str() {
                "flex-start" => Ok(AlignContent::FlexStart),
                "flex-end" => Ok(AlignContent::FlexEnd),
                "center" => Ok(AlignContent::Center),
                "space-between" => Ok(AlignContent::SpaceBetween),
                "space-around" => Ok(AlignContent::SpaceAround),
                "space-evenly" => Ok(AlignContent::SpaceEvenly),
                "stretch" => Ok(AlignContent::Stretch),
                _ => Ok(AlignContent::Stretch),
            },
            _ => Ok(AlignContent::Stretch),
        }
    }

    async fn create_flex_items(
        &self,
        children: &[NodeId],
        style_engine: &StyleEngine,
    ) -> std::result::Result<Vec<FlexItem>, LayoutError> {
        let mut items = Vec::new();

        for &child_id in children {
            if let Some(computed_styles) = style_engine.get_computed_styles(child_id) {
                let mut item = FlexItem::new(child_id);

                item.grow = match computed_styles.get_computed_value("flex-grow") {
                    Ok(ComputedValue::Length(v)) => v,
                    _ => 0.0,
                };
                item.shrink = match computed_styles.get_computed_value("flex-shrink") {
                    Ok(ComputedValue::Length(v)) => v,
                    _ => 0.0,
                };

                if let Ok(ComputedValue::Length(basis)) =
                    computed_styles.get_computed_value("flex-basis")
                {
                    item.basis = Some(basis);
                } else if let Ok(ComputedValue::Auto) =
                    computed_styles.get_computed_value("flex-basis")
                {
                    item.basis = None;
                }

                item.align_self = self.parse_align_self(&computed_styles)?;
                item.order = match computed_styles.get_computed_value("order") {
                    Ok(ComputedValue::Length(v)) => v,
                    _ => 0.0,
                } as i32;

                items.push(item);
            }
        }

        items.sort_by_key(|item| item.order);

        Ok(items)
    }

    fn parse_align_self(
        &self,
        styles: &ComputedStyles,
    ) -> std::result::Result<AlignSelf, LayoutError> {
        match styles.get_computed_value("align-self") {
            Ok(ComputedValue::Keyword(keyword)) => match keyword.as_str() {
                "auto" => Ok(AlignSelf::Auto),
                "flex-start" => Ok(AlignSelf::FlexStart),
                "flex-end" => Ok(AlignSelf::FlexEnd),
                "center" => Ok(AlignSelf::Center),
                "baseline" => Ok(AlignSelf::Baseline),
                "stretch" => Ok(AlignSelf::Stretch),
                _ => Ok(AlignSelf::Auto),
            },
            _ => Ok(AlignSelf::Auto),
        }
    }

    fn get_main_axis_size(
        &self,
        container: &FlexContainer,
        constraints: &LayoutConstraints,
    ) -> Option<f32> {
        match container.direction {
            FlexDirection::Row | FlexDirection::RowReverse => constraints.available_width,
            FlexDirection::Column | FlexDirection::ColumnReverse => constraints.available_height,
        }
    }

    fn get_cross_axis_size(
        &self,
        container: &FlexContainer,
        constraints: &LayoutConstraints,
    ) -> Option<f32> {
        match container.direction {
            FlexDirection::Row | FlexDirection::RowReverse => constraints.available_height,
            FlexDirection::Column | FlexDirection::ColumnReverse => constraints.available_width,
        }
    }

    async fn resolve_flex_base_sizes(
        &self,
        items: &mut [FlexItem],
        container: &FlexContainer,
        document: &Document,
        style_engine: &StyleEngine,
        layout_engine: &LayoutEngine,
        generation: u64,
    ) -> std::result::Result<(), LayoutError> {
        for item in items.iter_mut() {
            if let Some(basis) = item.basis {
                item.flex_base_size = basis;
            } else {
                let constraints = LayoutConstraints::default();

                let layout_result = layout_engine
                    .layout_node_public(
                        item.node_id,
                        constraints,
                        document,
                        style_engine,
                        generation,
                    )
                    .await?;

                item.flex_base_size = match container.direction {
                    FlexDirection::Row | FlexDirection::RowReverse => {
                        layout_result.layout_box.content_width
                    }
                    FlexDirection::Column | FlexDirection::ColumnReverse => {
                        layout_result.layout_box.content_height
                    }
                };

                item.layout_result = Some(layout_result);
            }

            item.hypothetical_main_size = item.flex_base_size;
        }

        Ok(())
    }

    fn collect_flex_lines(
        &self,
        items: &mut [FlexItem],
        container: &FlexContainer,
        container_main_size: Option<f32>,
    ) -> Vec<FlexLine> {
        let mut lines = Vec::new();

        if container.wrap == FlexWrap::NoWrap {
            let mut line = FlexLine::new();
            line.items = items.to_vec();
            lines.push(line);
        } else {
            let mut current_line = FlexLine::new();
            let mut current_main_size = 0.0;
            let available_main_size = container_main_size.unwrap_or(f32::INFINITY);

            for item in items.iter() {
                let item_main_size = item.hypothetical_main_size;
                let gap = if current_line.items.is_empty() {
                    0.0
                } else {
                    container.column_gap
                };

                if current_main_size + gap + item_main_size <= available_main_size
                    || current_line.items.is_empty()
                {
                    current_line.items.push(item.clone());
                    current_main_size += gap + item_main_size;
                } else {
                    current_line.main_size = current_main_size;
                    lines.push(current_line);

                    current_line = FlexLine::new();
                    current_line.items.push(item.clone());
                    current_main_size = item_main_size;
                }
            }

            if !current_line.items.is_empty() {
                current_line.main_size = current_main_size;
                lines.push(current_line);
            }
        }

        lines
    }

    fn resolve_flex_lengths(
        &self,
        lines: &mut [FlexLine],
        container: &FlexContainer,
        container_main_size: Option<f32>,
    ) -> Result<()> {
        for line in lines.iter_mut() {
            let available_main_size = container_main_size.unwrap_or_else(|| {
                line.items
                    .iter()
                    .map(|item| item.hypothetical_main_size)
                    .sum::<f32>()
            });

            let total_hypothetical_main_size: f32 = line
                .items
                .iter()
                .map(|item| item.hypothetical_main_size)
                .sum();

            let total_gap = if line.items.len() > 1 {
                (line.items.len() - 1) as f32 * container.column_gap
            } else {
                0.0
            };

            let free_space = available_main_size - total_hypothetical_main_size - total_gap;

            if free_space > 0.0 {
                self.grow_flex_items(&mut line.items, free_space)?;
            } else if free_space < 0.0 {
                self.shrink_flex_items(&mut line.items, -free_space)?;
            }

            for item in &mut line.items {
                item.target_main_size = item.hypothetical_main_size;
                item.main_size = item.target_main_size;
            }

            line.main_size = line.items.iter().map(|item| item.main_size).sum::<f32>() + total_gap;
        }

        Ok(())
    }

    fn grow_flex_items(&self, items: &mut [FlexItem], free_space: f32) -> Result<()> {
        let total_flex_grow: f32 = items.iter().map(|item| item.grow).sum();

        if total_flex_grow <= 0.0 {
            return Ok(());
        }

        let total_flex_grow_copy = total_flex_grow;
        for item in items.iter_mut() {
            if item.grow > 0.0 {
                let share = (item.grow / total_flex_grow_copy) * free_space;
                item.hypothetical_main_size = item.flex_base_size + share;
            }
        }

        Ok(())
    }

    fn shrink_flex_items(&self, items: &mut [FlexItem], negative_free_space: f32) -> Result<()> {
        let total_scaled_flex_shrink_factor: f32 = items
            .iter()
            .map(|item| item.flex_base_size * item.shrink)
            .sum();

        if total_scaled_flex_shrink_factor <= 0.0 {
            return Ok(());
        }

        for item in items.iter_mut() {
            item.scaled_flex_shrink_factor = item.flex_base_size * item.shrink;

            if item.scaled_flex_shrink_factor > 0.0 {
                let ratio = item.scaled_flex_shrink_factor / total_scaled_flex_shrink_factor;
                let share = ratio * negative_free_space;
                item.hypothetical_main_size = item.flex_base_size - share;
                item.hypothetical_main_size = item.hypothetical_main_size.max(0.0);
            }
        }

        Ok(())
    }

    async fn determine_cross_axis_sizes(
        &self,
        lines: &mut [FlexLine],
        container: &FlexContainer,
        document: &Document,
        style_engine: &StyleEngine,
        layout_engine: &LayoutEngine,
        generation: u64,
    ) -> std::result::Result<(), LayoutError> {
        for line in lines.iter_mut() {
            let mut max_cross_size = 0.0f32;
            let mut baseline = 0.0f32;

            for item in &mut line.items {
                let main_size = item.main_size;

                let constraints = match container.direction {
                    FlexDirection::Row | FlexDirection::RowReverse => LayoutConstraints {
                        available_width: Some(main_size),
                        available_height: None,
                        ..Default::default()
                    },
                    FlexDirection::Column | FlexDirection::ColumnReverse => LayoutConstraints {
                        available_width: None,
                        available_height: Some(main_size),
                        ..Default::default()
                    },
                };

                let layout_result = layout_engine
                    .layout_node_public(
                        item.node_id,
                        constraints,
                        document,
                        style_engine,
                        generation,
                    )
                    .await?;

                item.cross_size = match container.direction {
                    FlexDirection::Row | FlexDirection::RowReverse => {
                        layout_result.layout_box.content_height
                    }
                    FlexDirection::Column | FlexDirection::ColumnReverse => {
                        layout_result.layout_box.content_width
                    }
                };

                // Extract baseline before moving layout_result
                let item_baseline_opt = layout_result.baseline;

                item.layout_result = Some(layout_result);

                if item.align_self == AlignSelf::Baseline
                    || (item.align_self == AlignSelf::Auto
                        && container.align_items == AlignItems::Baseline)
                {
                    if let Some(item_baseline) = item_baseline_opt {
                        baseline = baseline.max(item_baseline);
                    }
                }

                max_cross_size = max_cross_size.max(item.cross_size);
            }

            line.cross_size = max_cross_size;
            line.baseline = baseline;
        }

        Ok(())
    }

    fn handle_align_content(
        &self,
        lines: &mut [FlexLine],
        container: &FlexContainer,
        container_cross_size: Option<f32>,
    ) {
        if lines.len() <= 1 || container_cross_size.is_none() {
            return;
        }

        let available_cross_size = container_cross_size.unwrap();
        let total_lines_cross_size: f32 = lines.iter().map(|line| line.cross_size).sum();
        let total_gap = if lines.len() > 1 {
            (lines.len() - 1) as f32 * container.row_gap
        } else {
            0.0
        };

        let free_space = available_cross_size - total_lines_cross_size - total_gap;

        let (offset, spacing) = match container.align_content {
            AlignContent::FlexStart => (0.0, 0.0),
            AlignContent::FlexEnd => (free_space, 0.0),
            AlignContent::Center => (free_space / 2.0, 0.0),
            AlignContent::SpaceBetween => {
                if lines.len() > 1 {
                    (0.0, free_space / (lines.len() - 1) as f32)
                } else {
                    (0.0, 0.0)
                }
            }
            AlignContent::SpaceAround => {
                let space = free_space / lines.len() as f32;
                (space / 2.0, space)
            }
            AlignContent::SpaceEvenly => {
                let space = free_space / (lines.len() + 1) as f32;
                (space, space)
            }
            AlignContent::Stretch => {
                if free_space > 0.0 {
                    let extra_per_line = free_space / lines.len() as f32;
                    for line in lines.iter_mut() {
                        line.cross_size += extra_per_line;
                    }
                }
                (0.0, 0.0)
            }
        };

        // Position lines with calculated offset and spacing
        let mut _current_position = offset;
        for line in lines.iter_mut() {
            // Store line position for later use in positioning items
            _current_position += line.cross_size + spacing + container.row_gap;
        }
    }

    fn position_flex_items(
        &self,
        lines: &mut [FlexLine],
        container: &FlexContainer,
        container_box: &LayoutBox,
    ) {
        let mut current_cross_position = container_box.content_y;

        for line in lines.iter_mut() {
            self.position_items_on_main_axis(
                &mut line.items,
                container,
                container_box.content_x,
                container_box.content_width,
            );
            self.position_items_on_cross_axis(
                &mut line.items,
                container,
                current_cross_position,
                line.cross_size,
                line.baseline,
            );

            current_cross_position += line.cross_size + container.row_gap;
        }
    }

    fn position_items_on_main_axis(
        &self,
        items: &mut [FlexItem],
        container: &FlexContainer,
        container_start: f32,
        container_main_size: f32,
    ) {
        let total_item_main_size: f32 = items.iter().map(|item| item.main_size).sum();
        let total_gap = if items.len() > 1 {
            (items.len() - 1) as f32 * container.column_gap
        } else {
            0.0
        };

        let free_space = container_main_size - total_item_main_size - total_gap;

        let (initial_offset, spacing) = match container.justify_content {
            JustifyContent::FlexStart => (0.0, 0.0),
            JustifyContent::FlexEnd => (free_space, 0.0),
            JustifyContent::Center => (free_space / 2.0, 0.0),
            JustifyContent::SpaceBetween => {
                if items.len() > 1 {
                    (0.0, free_space / (items.len() - 1) as f32)
                } else {
                    (0.0, 0.0)
                }
            }
            JustifyContent::SpaceAround => {
                let space = free_space / items.len() as f32;
                (space / 2.0, space)
            }
            JustifyContent::SpaceEvenly => {
                let space = free_space / (items.len() + 1) as f32;
                (space, space)
            }
        };

        let mut current_position = container_start + initial_offset;
        let items_len = items.len();

        for (i, item) in items.iter_mut().enumerate() {
            let item_position = match container.direction {
                FlexDirection::Row => current_position,
                FlexDirection::RowReverse => {
                    container_start + container_main_size - current_position - item.main_size
                }
                FlexDirection::Column => current_position,
                FlexDirection::ColumnReverse => {
                    container_start + container_main_size - current_position - item.main_size
                }
            };

            if let Some(ref mut layout_result) = item.layout_result {
                match container.direction {
                    FlexDirection::Row | FlexDirection::RowReverse => {
                        layout_result.layout_box.content_x = item_position;
                    }
                    FlexDirection::Column | FlexDirection::ColumnReverse => {
                        layout_result.layout_box.content_y = item_position;
                    }
                }
            }

            current_position += item.main_size + spacing;
            if i < items_len - 1 {
                current_position += container.column_gap;
            }
        }
    }

    fn position_items_on_cross_axis(
        &self,
        items: &mut [FlexItem],
        container: &FlexContainer,
        line_cross_start: f32,
        line_cross_size: f32,
        line_baseline: f32,
    ) {
        for item in items.iter_mut() {
            let align = if item.align_self == AlignSelf::Auto {
                match container.align_items {
                    AlignItems::FlexStart => AlignSelf::FlexStart,
                    AlignItems::FlexEnd => AlignSelf::FlexEnd,
                    AlignItems::Center => AlignSelf::Center,
                    AlignItems::Baseline => AlignSelf::Baseline,
                    AlignItems::Stretch => AlignSelf::Stretch,
                }
            } else {
                item.align_self
            };

            let item_cross_position = match align {
                AlignSelf::FlexStart => line_cross_start,
                AlignSelf::FlexEnd => line_cross_start + line_cross_size - item.cross_size,
                AlignSelf::Center => line_cross_start + (line_cross_size - item.cross_size) / 2.0,
                AlignSelf::Baseline => {
                    if let Some(ref layout_result) = item.layout_result {
                        if let Some(item_baseline) = layout_result.baseline {
                            line_cross_start + line_baseline - item_baseline
                        } else {
                            line_cross_start
                        }
                    } else {
                        line_cross_start
                    }
                }
                AlignSelf::Stretch => {
                    item.cross_size = line_cross_size;
                    line_cross_start
                }
                AlignSelf::Auto => line_cross_start,
            };

            if let Some(ref mut layout_result) = item.layout_result {
                match container.direction {
                    FlexDirection::Row | FlexDirection::RowReverse => {
                        layout_result.layout_box.content_y = item_cross_position;
                        if align == AlignSelf::Stretch {
                            layout_result.layout_box.content_height = item.cross_size;
                        }
                    }
                    FlexDirection::Column | FlexDirection::ColumnReverse => {
                        layout_result.layout_box.content_x = item_cross_position;
                        if align == AlignSelf::Stretch {
                            layout_result.layout_box.content_width = item.cross_size;
                        }
                    }
                }
            }
        }
    }

    fn compute_container_box(
        &self,
        computed_styles: &ComputedStyles,
        constraints: &LayoutConstraints,
    ) -> std::result::Result<LayoutBox, LayoutError> {
        let width = constraints.available_width.unwrap_or(0.0);
        let height = constraints.available_height.unwrap_or(0.0);

        let padding_top = match computed_styles.get_computed_value("padding_top") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let padding_right = match computed_styles.get_computed_value("padding_right") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let padding_bottom = match computed_styles.get_computed_value("padding_bottom") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let padding_left = match computed_styles.get_computed_value("padding_left") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };

        let border_top = match computed_styles.get_computed_value("border-top-width") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let border_right = match computed_styles.get_computed_value("border-right-width") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let border_bottom = match computed_styles.get_computed_value("border-bottom-width") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let border_left = match computed_styles.get_computed_value("border-left-width") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };

        let margin_top = match computed_styles.get_computed_value("margin_top") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let margin_right = match computed_styles.get_computed_value("margin_right") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let margin_bottom = match computed_styles.get_computed_value("margin_bottom") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        let margin_left = match computed_styles.get_computed_value("margin_left") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };

        let content_width = width - padding_left - padding_right - border_left - border_right;
        let content_height = height - padding_top - padding_bottom - border_top - border_bottom;

        Ok(LayoutBox {
            content_x: margin_left + border_left + padding_left,
            content_y: margin_top + border_top + padding_top,
            content_width: content_width.max(0.0),
            content_height: content_height.max(0.0),
            padding_top,
            padding_right,
            padding_bottom,
            padding_left,
            border_top,
            border_right,
            border_bottom,
            border_left,
            margin_top,
            margin_right,
            margin_bottom,
            margin_left,
        })
    }

    pub fn clear_cache(&self) {
        self.cache.clear();
    }
}
