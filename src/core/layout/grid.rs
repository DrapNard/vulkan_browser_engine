use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

use super::engine::{LayoutBox, LayoutConstraints, LayoutEngine, LayoutError, LayoutResult};
use crate::core::{
    css::{ComputedStyles, ComputedValue, StyleEngine},
    dom::{Document, NodeId},
};

#[derive(Error, Debug)]
pub enum GridError {
    #[error("Grid computation failed: {0}")]
    Computation(String),
    #[error("Invalid grid value: {0}")]
    InvalidGridValue(String),
    #[error("Grid item placement failed: {0}")]
    ItemPlacement(String),
    #[error("Track sizing failed: {0}")]
    TrackSizing(String),
}

pub type Result<T> = std::result::Result<T, GridError>;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum TrackSize {
    Length(f32),
    Percentage(f32),
    Fr(f32),
    MinContent,
    MaxContent,
    #[default]
    Auto,
    MinMax(Box<TrackSize>, Box<TrackSize>),
    FitContent(Box<TrackSize>),
}

#[derive(Debug, Clone)]
pub struct GridTrack {
    pub size: TrackSize,
    pub min_size: f32,
    pub max_size: f32,
    pub base_size: f32,
    pub growth_limit: f32,
    pub planned_increase: f32,
    pub item_incurred_increase: f32,
    pub infinity_increased: bool,
}

impl Default for GridTrack {
    fn default() -> Self {
        Self {
            size: TrackSize::Auto,
            min_size: 0.0,
            max_size: f32::INFINITY,
            base_size: 0.0,
            growth_limit: f32::INFINITY,
            planned_increase: 0.0,
            item_incurred_increase: 0.0,
            infinity_increased: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GridLine {
    Line(i32),
    Span(u32),
    #[default]
    Auto,
}

#[derive(Debug, Clone)]
pub struct GridArea {
    pub row_start: GridLine,
    pub row_end: GridLine,
    pub column_start: GridLine,
    pub column_end: GridLine,
}

impl Default for GridArea {
    fn default() -> Self {
        Self {
            row_start: GridLine::Auto,
            row_end: GridLine::Auto,
            column_start: GridLine::Auto,
            column_end: GridLine::Auto,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GridItem {
    pub node_id: NodeId,
    pub area: GridArea,
    pub resolved_area: ResolvedGridArea,
    pub layout_result: Option<LayoutResult>,
    pub order: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ResolvedGridArea {
    pub row_start: u32,
    pub row_end: u32,
    pub column_start: u32,
    pub column_end: u32,
}

impl ResolvedGridArea {
    pub fn row_span(&self) -> u32 {
        self.row_end.saturating_sub(self.row_start)
    }

    pub fn column_span(&self) -> u32 {
        self.column_end.saturating_sub(self.column_start)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JustifyItems {
    Start,
    End,
    Center,
    #[default]
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignItems {
    Start,
    End,
    Center,
    #[default]
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JustifyContent {
    #[default]
    Start,
    End,
    Center,
    Stretch,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignContent {
    #[default]
    Start,
    End,
    Center,
    Stretch,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Debug, Clone)]
pub struct GridContainer {
    pub row_tracks: Vec<GridTrack>,
    pub column_tracks: Vec<GridTrack>,
    pub row_gap: f32,
    pub column_gap: f32,
    pub justify_items: JustifyItems,
    pub align_items: AlignItems,
    pub justify_content: JustifyContent,
    pub align_content: AlignContent,
    pub implicit_row_size: TrackSize,
    pub implicit_column_size: TrackSize,
    pub auto_flow: GridAutoFlow,
    pub dense: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GridAutoFlow {
    #[default]
    Row,
    Column,
}

impl Default for GridContainer {
    fn default() -> Self {
        Self {
            row_tracks: Vec::new(),
            column_tracks: Vec::new(),
            row_gap: 0.0,
            column_gap: 0.0,
            justify_items: JustifyItems::default(),
            align_items: AlignItems::default(),
            justify_content: JustifyContent::default(),
            align_content: AlignContent::default(),
            implicit_row_size: TrackSize::Auto,
            implicit_column_size: TrackSize::Auto,
            auto_flow: GridAutoFlow::default(),
            dense: false,
        }
    }
}

pub struct GridLayout {
    cache: Arc<dashmap::DashMap<NodeId, GridContainer>>,
}

impl Default for GridLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl GridLayout {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(dashmap::DashMap::new()),
        }
    }

    pub async fn layout_grid_container(
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

        let mut grid_container = self.parse_grid_container(&computed_styles)?;

        let children = document.get_children(node_id);
        let mut grid_items = self
            .create_grid_items(&children, document, style_engine)
            .await?;

        self.resolve_explicit_grid(&mut grid_container, &computed_styles)?;

        self.place_grid_items(&mut grid_container, &mut grid_items)?;

        self.size_grid_tracks(
            &mut grid_container,
            &grid_items,
            &constraints,
            document,
            style_engine,
            layout_engine,
            generation,
        )
        .await?;

        self.align_and_position_items(
            &grid_container,
            &mut grid_items,
            document,
            style_engine,
            layout_engine,
            generation,
        )
        .await?;

        let layout_box =
            self.compute_container_box(&computed_styles, &constraints, &grid_container)?;

        let grid_width = self.calculate_grid_width(&grid_container);
        let grid_height = self.calculate_grid_height(&grid_container);

        let children_overflow =
            grid_width > layout_box.content_width || grid_height > layout_box.content_height;

        self.cache.insert(node_id, grid_container);

        Ok(LayoutResult {
            layout_box,
            baseline: Some(layout_box.content_y + layout_box.content_height),
            intrinsic_width: grid_width,
            intrinsic_height: grid_height,
            children_overflow,
        })
    }

    fn parse_grid_container(
        &self,
        styles: &ComputedStyles,
    ) -> std::result::Result<GridContainer, LayoutError> {
        let mut container = GridContainer::default();

        container.row_gap = match styles.get_computed_value("row-gap") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };
        container.column_gap = match styles.get_computed_value("column-gap") {
            Ok(ComputedValue::Length(v)) => v,
            _ => 0.0,
        };

        if let Ok(gap) = styles.get_computed_value("gap") {
            if let ComputedValue::Length(gap_val) = gap {
                if container.row_gap == 0.0 {
                    container.row_gap = gap_val;
                }
                if container.column_gap == 0.0 {
                    container.column_gap = gap_val;
                }
            }
        }

        container.justify_items = self.parse_justify_items(styles)?;
        container.align_items = self.parse_align_items(styles)?;
        container.justify_content = self.parse_justify_content(styles)?;
        container.align_content = self.parse_align_content(styles)?;

        container.auto_flow = self.parse_grid_auto_flow(styles)?;
        container.dense = self.parse_grid_auto_flow_dense(styles);

        Ok(container)
    }

    fn parse_justify_items(
        &self,
        styles: &ComputedStyles,
    ) -> std::result::Result<JustifyItems, LayoutError> {
        match styles.get_computed_value("justify-items") {
            Ok(ComputedValue::Keyword(keyword)) => match keyword.as_str() {
                "start" => Ok(JustifyItems::Start),
                "end" => Ok(JustifyItems::End),
                "center" => Ok(JustifyItems::Center),
                "stretch" => Ok(JustifyItems::Stretch),
                _ => Ok(JustifyItems::Stretch),
            },
            _ => Ok(JustifyItems::Stretch),
        }
    }

    fn parse_align_items(
        &self,
        styles: &ComputedStyles,
    ) -> std::result::Result<AlignItems, LayoutError> {
        match styles.get_computed_value("align-items") {
            Ok(ComputedValue::Keyword(keyword)) => match keyword.as_str() {
                "start" => Ok(AlignItems::Start),
                "end" => Ok(AlignItems::End),
                "center" => Ok(AlignItems::Center),
                "stretch" => Ok(AlignItems::Stretch),
                _ => Ok(AlignItems::Stretch),
            },
            _ => Ok(AlignItems::Stretch),
        }
    }

    fn parse_justify_content(
        &self,
        styles: &ComputedStyles,
    ) -> std::result::Result<JustifyContent, LayoutError> {
        match styles.get_computed_value("justify-content") {
            Ok(ComputedValue::Keyword(keyword)) => match keyword.as_str() {
                "start" => Ok(JustifyContent::Start),
                "end" => Ok(JustifyContent::End),
                "center" => Ok(JustifyContent::Center),
                "stretch" => Ok(JustifyContent::Stretch),
                "space-between" => Ok(JustifyContent::SpaceBetween),
                "space-around" => Ok(JustifyContent::SpaceAround),
                "space-evenly" => Ok(JustifyContent::SpaceEvenly),
                _ => Ok(JustifyContent::Start),
            },
            _ => Ok(JustifyContent::Start),
        }
    }

    fn parse_align_content(
        &self,
        styles: &ComputedStyles,
    ) -> std::result::Result<AlignContent, LayoutError> {
        match styles.get_computed_value("align-content") {
            Ok(ComputedValue::Keyword(keyword)) => match keyword.as_str() {
                "start" => Ok(AlignContent::Start),
                "end" => Ok(AlignContent::End),
                "center" => Ok(AlignContent::Center),
                "stretch" => Ok(AlignContent::Stretch),
                "space-between" => Ok(AlignContent::SpaceBetween),
                "space-around" => Ok(AlignContent::SpaceAround),
                "space-evenly" => Ok(AlignContent::SpaceEvenly),
                _ => Ok(AlignContent::Start),
            },
            _ => Ok(AlignContent::Start),
        }
    }

    fn parse_grid_auto_flow(
        &self,
        styles: &ComputedStyles,
    ) -> std::result::Result<GridAutoFlow, LayoutError> {
        match styles.get_computed_value("grid-auto-flow") {
            Ok(ComputedValue::Keyword(keyword)) => {
                if keyword.contains("column") {
                    Ok(GridAutoFlow::Column)
                } else {
                    Ok(GridAutoFlow::Row)
                }
            }
            _ => Ok(GridAutoFlow::Row),
        }
    }

    fn parse_grid_auto_flow_dense(&self, styles: &ComputedStyles) -> bool {
        match styles.get_computed_value("grid-auto-flow") {
            Ok(ComputedValue::Keyword(keyword)) => keyword.contains("dense"),
            _ => false,
        }
    }

    async fn create_grid_items(
        &self,
        children: &[NodeId],
        _document: &Document,
        style_engine: &StyleEngine,
    ) -> std::result::Result<Vec<GridItem>, LayoutError> {
        let mut items = Vec::new();

        for &child_id in children {
            if let Some(computed_styles) = style_engine.get_computed_styles(child_id) {
                let area = self.parse_grid_area(&computed_styles)?;
                let order = match computed_styles.get_computed_value("order") {
                    Ok(ComputedValue::Length(v)) => v,
                    _ => 0.0,
                } as i32;

                items.push(GridItem {
                    node_id: child_id,
                    area,
                    resolved_area: ResolvedGridArea::default(),
                    layout_result: None,
                    order,
                });
            }
        }

        items.sort_by_key(|item| item.order);

        Ok(items)
    }

    fn parse_grid_area(
        &self,
        styles: &ComputedStyles,
    ) -> std::result::Result<GridArea, LayoutError> {
        let mut area = GridArea::default();

        if let Ok(value) = styles.get_computed_value("grid-area") {
            // Parse shorthand grid-area property
            area = self.parse_grid_area_shorthand(&value)?;
        } else {
            area.row_start = self.parse_grid_line(styles, "grid-row-start")?;
            area.row_end = self.parse_grid_line(styles, "grid-row-end")?;
            area.column_start = self.parse_grid_line(styles, "grid-column-start")?;
            area.column_end = self.parse_grid_line(styles, "grid-column-end")?;
        }

        Ok(area)
    }

    fn parse_grid_area_shorthand(
        &self,
        _value: &ComputedValue,
    ) -> std::result::Result<GridArea, LayoutError> {
        // Simplified parsing - in a real implementation, this would be more comprehensive
        Ok(GridArea::default())
    }

    fn parse_grid_line(
        &self,
        styles: &ComputedStyles,
        property: &str,
    ) -> std::result::Result<GridLine, LayoutError> {
        match styles.get_computed_value(property) {
            Ok(ComputedValue::Integer(line)) => Ok(GridLine::Line(line)),
            Ok(ComputedValue::Keyword(keyword)) => {
                if keyword == "auto" {
                    Ok(GridLine::Auto)
                } else if keyword.starts_with("span ") {
                    if let Ok(span) = keyword.trim_start_matches("span ").parse::<u32>() {
                        Ok(GridLine::Span(span))
                    } else {
                        Ok(GridLine::Auto)
                    }
                } else {
                    Ok(GridLine::Auto)
                }
            }
            _ => Ok(GridLine::Auto),
        }
    }

    fn resolve_explicit_grid(
        &self,
        container: &mut GridContainer,
        styles: &ComputedStyles,
    ) -> std::result::Result<(), LayoutError> {
        if let Ok(value) = styles.get_computed_value("grid-template-rows") {
            container.row_tracks = self.parse_track_list(&value)?;
        }

        if let Ok(value) = styles.get_computed_value("grid-template-columns") {
            container.column_tracks = self.parse_track_list(&value)?;
        }

        if let Ok(value) = styles.get_computed_value("grid-auto-rows") {
            container.implicit_row_size = self.parse_track_size(&value)?;
        }

        if let Ok(value) = styles.get_computed_value("grid-auto-columns") {
            container.implicit_column_size = self.parse_track_size(&value)?;
        }

        Ok(())
    }

    fn parse_track_list(
        &self,
        value: &ComputedValue,
    ) -> std::result::Result<Vec<GridTrack>, LayoutError> {
        let mut tracks = Vec::new();

        match value {
            ComputedValue::List(values) => {
                for val in values {
                    let size = self.parse_track_size(val)?;
                    tracks.push(GridTrack {
                        size,
                        ..GridTrack::default()
                    });
                }
            }
            _ => {
                let size = self.parse_track_size(value)?;
                tracks.push(GridTrack {
                    size,
                    ..GridTrack::default()
                });
            }
        }

        Ok(tracks)
    }

    fn parse_track_size(
        &self,
        value: &ComputedValue,
    ) -> std::result::Result<TrackSize, LayoutError> {
        match value {
            ComputedValue::Length(length) => Ok(TrackSize::Length(*length)),
            ComputedValue::Percentage(percentage) => Ok(TrackSize::Percentage(*percentage)),
            ComputedValue::Keyword(keyword) => match keyword.as_str() {
                "auto" => Ok(TrackSize::Auto),
                "min-content" => Ok(TrackSize::MinContent),
                "max-content" => Ok(TrackSize::MaxContent),
                _ => {
                    if keyword.ends_with("fr") {
                        if let Ok(fr) = keyword.trim_end_matches("fr").parse::<f32>() {
                            Ok(TrackSize::Fr(fr))
                        } else {
                            Ok(TrackSize::Auto)
                        }
                    } else {
                        Ok(TrackSize::Auto)
                    }
                }
            },
            ComputedValue::Function { name, args } => match name.as_str() {
                "minmax" => {
                    if args.len() == 2 {
                        let min_size = self.parse_track_size(&args[0])?;
                        let max_size = self.parse_track_size(&args[1])?;
                        Ok(TrackSize::MinMax(Box::new(min_size), Box::new(max_size)))
                    } else {
                        Ok(TrackSize::Auto)
                    }
                }
                "fit-content" => {
                    if args.len() == 1 {
                        let size = self.parse_track_size(&args[0])?;
                        Ok(TrackSize::FitContent(Box::new(size)))
                    } else {
                        Ok(TrackSize::Auto)
                    }
                }
                _ => Ok(TrackSize::Auto),
            },
            _ => Ok(TrackSize::Auto),
        }
    }

    fn place_grid_items(
        &self,
        container: &mut GridContainer,
        items: &mut [GridItem],
    ) -> std::result::Result<(), LayoutError> {
        let mut placement_grid = PlacementGrid::new();

        for item in items.iter_mut() {
            let resolved = self.resolve_grid_area(&item.area, container, &placement_grid)?;
            item.resolved_area = resolved;

            placement_grid.place_item(resolved);

            self.expand_grid_if_needed(container, &resolved);
        }

        Ok(())
    }

    fn resolve_grid_area(
        &self,
        area: &GridArea,
        container: &GridContainer,
        _placement_grid: &PlacementGrid,
    ) -> std::result::Result<ResolvedGridArea, LayoutError> {
        let mut resolved = ResolvedGridArea::default();

        resolved.row_start =
            self.resolve_grid_line(&area.row_start, container.row_tracks.len(), true)?;
        resolved.row_end =
            self.resolve_grid_line(&area.row_end, container.row_tracks.len(), true)?;
        resolved.column_start =
            self.resolve_grid_line(&area.column_start, container.column_tracks.len(), false)?;
        resolved.column_end =
            self.resolve_grid_line(&area.column_end, container.column_tracks.len(), false)?;

        if resolved.row_start >= resolved.row_end {
            resolved.row_end = resolved.row_start + 1;
        }

        if resolved.column_start >= resolved.column_end {
            resolved.column_end = resolved.column_start + 1;
        }

        Ok(resolved)
    }

    fn resolve_grid_line(
        &self,
        line: &GridLine,
        track_count: usize,
        _is_row: bool,
    ) -> std::result::Result<u32, LayoutError> {
        match line {
            GridLine::Line(line_num) => {
                if *line_num > 0 {
                    Ok((*line_num as u32).saturating_sub(1))
                } else if *line_num < 0 {
                    let from_end = (-*line_num) as u32;
                    Ok((track_count as u32).saturating_sub(from_end))
                } else {
                    Ok(0)
                }
            }
            GridLine::Span(span) => Ok(*span),
            GridLine::Auto => Ok(0),
        }
    }

    fn expand_grid_if_needed(&self, container: &mut GridContainer, area: &ResolvedGridArea) {
        while container.row_tracks.len() < area.row_end as usize {
            container.row_tracks.push(GridTrack {
                size: container.implicit_row_size.clone(),
                ..GridTrack::default()
            });
        }

        while container.column_tracks.len() < area.column_end as usize {
            container.column_tracks.push(GridTrack {
                size: container.implicit_column_size.clone(),
                ..GridTrack::default()
            });
        }
    }

    async fn size_grid_tracks(
        &self,
        container: &mut GridContainer,
        items: &[GridItem],
        constraints: &LayoutConstraints,
        document: &Document,
        style_engine: &StyleEngine,
        layout_engine: &LayoutEngine,
        generation: u64,
    ) -> std::result::Result<(), LayoutError> {
        let available_width = constraints.available_width.unwrap_or(f32::INFINITY);
        let available_height = constraints.available_height.unwrap_or(f32::INFINITY);

        self.initialize_track_sizes(&mut container.column_tracks, available_width);
        self.initialize_track_sizes(&mut container.row_tracks, available_height);

        self.resolve_intrinsic_track_sizes(
            container,
            items,
            document,
            style_engine,
            layout_engine,
            generation,
        )
        .await?;

        self.maximize_tracks(&mut container.column_tracks);
        self.maximize_tracks(&mut container.row_tracks);

        self.expand_flexible_tracks(&mut container.column_tracks, available_width);
        self.expand_flexible_tracks(&mut container.row_tracks, available_height);

        Ok(())
    }

    fn initialize_track_sizes(&self, tracks: &mut [GridTrack], available_space: f32) {
        for track in tracks.iter_mut() {
            match &track.size {
                TrackSize::Length(length) => {
                    track.base_size = *length;
                    track.growth_limit = *length;
                }
                TrackSize::Percentage(percentage) => {
                    let size = available_space * percentage / 100.0;
                    track.base_size = size;
                    track.growth_limit = size;
                }
                TrackSize::MinContent | TrackSize::MaxContent | TrackSize::Auto => {
                    track.base_size = 0.0;
                    track.growth_limit = f32::INFINITY;
                }
                TrackSize::Fr(_) => {
                    track.base_size = 0.0;
                    track.growth_limit = f32::INFINITY;
                }
                TrackSize::MinMax(min_size, max_size) => {
                    track.base_size = self.resolve_track_size_value(min_size, available_space);
                    track.growth_limit = self.resolve_track_size_value(max_size, available_space);
                }
                TrackSize::FitContent(size) => {
                    let content_size = self.resolve_track_size_value(size, available_space);
                    track.base_size = 0.0;
                    track.growth_limit = content_size;
                }
            }
        }
    }

    fn resolve_track_size_value(&self, size: &TrackSize, available_space: f32) -> f32 {
        match size {
            TrackSize::Length(length) => *length,
            TrackSize::Percentage(percentage) => available_space * percentage / 100.0,
            TrackSize::Auto | TrackSize::MinContent | TrackSize::MaxContent => 0.0,
            TrackSize::Fr(_) => f32::INFINITY,
            _ => 0.0,
        }
    }

    async fn resolve_intrinsic_track_sizes(
        &self,
        container: &mut GridContainer,
        items: &[GridItem],
        document: &Document,
        style_engine: &StyleEngine,
        layout_engine: &LayoutEngine,
        generation: u64,
    ) -> std::result::Result<(), LayoutError> {
        // Simplified intrinsic sizing - real implementation would be more comprehensive
        for item in items {
            let constraints = LayoutConstraints::default();

            if let Ok(layout_result) = layout_engine
                .layout_node_public(
                    item.node_id,
                    constraints,
                    document,
                    style_engine,
                    generation,
                )
                .await
            {
                let area = &item.resolved_area;

                if area.column_span() == 1
                    && (area.column_start as usize) < container.column_tracks.len()
                {
                    let track = &mut container.column_tracks[area.column_start as usize];
                    track.base_size = track.base_size.max(layout_result.layout_box.content_width);
                }

                if area.row_span() == 1 && (area.row_start as usize) < container.row_tracks.len() {
                    let track = &mut container.row_tracks[area.row_start as usize];
                    track.base_size = track.base_size.max(layout_result.layout_box.content_height);
                }
            }
        }

        Ok(())
    }

    fn maximize_tracks(&self, tracks: &mut [GridTrack]) {
        for track in tracks.iter_mut() {
            if track.growth_limit == f32::INFINITY {
                track.growth_limit = track.base_size;
            }
        }
    }

    fn expand_flexible_tracks(&self, tracks: &mut [GridTrack], available_space: f32) {
        let fixed_space: f32 = tracks
            .iter()
            .filter(|t| !matches!(t.size, TrackSize::Fr(_)))
            .map(|t| t.base_size)
            .sum();

        let remaining_space = (available_space - fixed_space).max(0.0);

        let total_fr: f32 = tracks
            .iter()
            .filter_map(|t| match &t.size {
                TrackSize::Fr(fr) => Some(*fr),
                _ => None,
            })
            .sum();

        if total_fr > 0.0 {
            let fr_size = remaining_space / total_fr;

            for track in tracks.iter_mut() {
                if let TrackSize::Fr(fr) = &track.size {
                    track.base_size = fr * fr_size;
                    track.growth_limit = track.base_size;
                }
            }
        }
    }

    async fn align_and_position_items(
        &self,
        container: &GridContainer,
        items: &mut [GridItem],
        document: &Document,
        style_engine: &StyleEngine,
        layout_engine: &LayoutEngine,
        generation: u64,
    ) -> std::result::Result<(), LayoutError> {
        for item in items.iter_mut() {
            let item_constraints = self.calculate_item_constraints(container, &item.resolved_area);

            let layout_result = layout_engine
                .layout_node_public(
                    item.node_id,
                    item_constraints,
                    document,
                    style_engine,
                    generation,
                )
                .await?;

            let position = self.calculate_item_position(
                container,
                &item.resolved_area,
                &layout_result.layout_box,
            );

            let mut positioned_result = layout_result;
            positioned_result.layout_box.content_x = position.0;
            positioned_result.layout_box.content_y = position.1;

            item.layout_result = Some(positioned_result);
        }

        Ok(())
    }

    fn calculate_item_constraints(
        &self,
        container: &GridContainer,
        area: &ResolvedGridArea,
    ) -> LayoutConstraints {
        let mut width = 0.0;
        let mut height = 0.0;

        for i in area.column_start..area.column_end {
            if let Some(track) = container.column_tracks.get(i as usize) {
                width += track.base_size;
            }
            if i > area.column_start {
                width += container.column_gap;
            }
        }

        for i in area.row_start..area.row_end {
            if let Some(track) = container.row_tracks.get(i as usize) {
                height += track.base_size;
            }
            if i > area.row_start {
                height += container.row_gap;
            }
        }

        LayoutConstraints {
            available_width: Some(width),
            available_height: Some(height),
            ..Default::default()
        }
    }

    fn calculate_item_position(
        &self,
        container: &GridContainer,
        area: &ResolvedGridArea,
        item_box: &LayoutBox,
    ) -> (f32, f32) {
        let mut x = 0.0;
        let mut y = 0.0;

        for i in 0..area.column_start {
            if let Some(track) = container.column_tracks.get(i as usize) {
                x += track.base_size;
            }
            x += container.column_gap;
        }

        for i in 0..area.row_start {
            if let Some(track) = container.row_tracks.get(i as usize) {
                y += track.base_size;
            }
            y += container.row_gap;
        }

        let cell_width = self.calculate_cell_width(container, area);
        let cell_height = self.calculate_cell_height(container, area);

        match container.justify_items {
            JustifyItems::Start => {}
            JustifyItems::End => x += cell_width - item_box.content_width,
            JustifyItems::Center => x += (cell_width - item_box.content_width) / 2.0,
            JustifyItems::Stretch => {}
        }

        match container.align_items {
            AlignItems::Start => {}
            AlignItems::End => y += cell_height - item_box.content_height,
            AlignItems::Center => y += (cell_height - item_box.content_height) / 2.0,
            AlignItems::Stretch => {}
        }

        (x, y)
    }

    fn calculate_cell_width(&self, container: &GridContainer, area: &ResolvedGridArea) -> f32 {
        let mut width = 0.0;
        for i in area.column_start..area.column_end {
            if let Some(track) = container.column_tracks.get(i as usize) {
                width += track.base_size;
            }
            if i > area.column_start {
                width += container.column_gap;
            }
        }
        width
    }

    fn calculate_cell_height(&self, container: &GridContainer, area: &ResolvedGridArea) -> f32 {
        let mut height = 0.0;
        for i in area.row_start..area.row_end {
            if let Some(track) = container.row_tracks.get(i as usize) {
                height += track.base_size;
            }
            if i > area.row_start {
                height += container.row_gap;
            }
        }
        height
    }

    fn compute_container_box(
        &self,
        computed_styles: &ComputedStyles,
        constraints: &LayoutConstraints,
        container: &GridContainer,
    ) -> std::result::Result<LayoutBox, LayoutError> {
        let grid_width = self.calculate_grid_width(container);
        let grid_height = self.calculate_grid_height(container);

        let content_width = constraints.available_width.unwrap_or(grid_width);
        let content_height = constraints.available_height.unwrap_or(grid_height);

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

        Ok(LayoutBox {
            content_x: margin_left + border_left + padding_left,
            content_y: margin_top + border_top + padding_top,
            content_width,
            content_height,
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

    fn calculate_grid_width(&self, container: &GridContainer) -> f32 {
        let tracks_width: f32 = container.column_tracks.iter().map(|t| t.base_size).sum();
        let gaps_width = if container.column_tracks.len() > 1 {
            (container.column_tracks.len() - 1) as f32 * container.column_gap
        } else {
            0.0
        };
        tracks_width + gaps_width
    }

    fn calculate_grid_height(&self, container: &GridContainer) -> f32 {
        let tracks_height: f32 = container.row_tracks.iter().map(|t| t.base_size).sum();
        let gaps_height = if container.row_tracks.len() > 1 {
            (container.row_tracks.len() - 1) as f32 * container.row_gap
        } else {
            0.0
        };
        tracks_height + gaps_height
    }

    pub fn clear_cache(&self) {
        self.cache.clear();
    }
}

struct PlacementGrid {
    occupied: HashMap<(u32, u32), bool>,
}

impl PlacementGrid {
    fn new() -> Self {
        Self {
            occupied: HashMap::new(),
        }
    }

    fn place_item(&mut self, area: ResolvedGridArea) {
        for row in area.row_start..area.row_end {
            for col in area.column_start..area.column_end {
                self.occupied.insert((row, col), true);
            }
        }
    }
}
