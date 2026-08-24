//! Live Session Usage projection, receipt navigation, and Surface rendering.

use std::num::NonZeroU16;

use yo_core::{SessionUsageError, SessionUsageProjection, SessionUsageReceipt, TranscriptRecord};

use crate::{
    appearance::AppearanceSnapshot,
    input::event::KeyCode,
    overlay::SelectionPanelGlyphs,
    runner::usage_format::{aggregate_text, cache_read_text, safe_text, source_text, value_text},
    surface::{Point, Rect, Style, SurfaceView, WriteOutcome},
    text::flow::{TextFlowError, flow_text},
};

const GUTTER_WIDTH: u16 = 2;
const WIDE_LAYOUT_MIN_WIDTH: u16 = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UsageNavigation {
    Previous,
    Next,
    PageUp,
    PageDown,
    Home,
    End,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct UsageViewState {
    selected: usize,
    first_visible: usize,
    pending: Option<UsageNavigation>,
}

impl UsageViewState {
    pub(super) fn queue(&mut self, navigation: UsageNavigation) {
        self.pending = Some(navigation);
    }

    fn reconcile(&mut self, receipt_count: usize, viewport: usize) {
        let viewport = viewport.max(1);
        if receipt_count == 0 {
            self.selected = 0;
            self.first_visible = 0;
            self.pending = None;
            return;
        }

        self.selected = self.selected.min(receipt_count - 1);
        if let Some(navigation) = self.pending.take() {
            let page = viewport;
            self.selected = match navigation {
                UsageNavigation::Previous => self.selected.saturating_sub(1),
                UsageNavigation::Next => (self.selected + 1).min(receipt_count - 1),
                UsageNavigation::PageUp => self.selected.saturating_sub(page),
                UsageNavigation::PageDown => {
                    self.selected.saturating_add(page).min(receipt_count - 1)
                },
                UsageNavigation::Home => 0,
                UsageNavigation::End => receipt_count - 1,
            };
        }

        if self.selected < self.first_visible {
            self.first_visible = self.selected;
        } else if self.selected >= self.first_visible.saturating_add(viewport) {
            self.first_visible = self.selected.saturating_sub(viewport - 1);
        }
        self.first_visible = self
            .first_visible
            .min(receipt_count.saturating_sub(viewport));
    }

    #[cfg(test)]
    pub(super) const fn position(self) -> (usize, usize) {
        (self.selected, self.first_visible)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runner) enum UsageRenderError {
    WidthUnavailable,
    HeightUnavailable,
    Text(TextFlowError),
    SurfaceConflict,
}

#[derive(Clone, Debug)]
pub(super) struct UsageView {
    projection: Option<SessionUsageProjection>,
    error: Option<SessionUsageError>,
}

impl Default for UsageView {
    fn default() -> Self {
        let projection = SessionUsageProjection::from_records(&[])
            .expect("an empty semantic record set has a valid empty Usage projection");
        Self {
            projection: Some(projection),
            error: None,
        }
    }
}

impl UsageView {
    pub(super) fn observe_records(&mut self, records: &[TranscriptRecord]) {
        match SessionUsageProjection::from_records(records) {
            Ok(projection) => {
                self.projection = Some(projection);
                self.error = None;
            },
            Err(error) => {
                self.projection = None;
                self.error = Some(error);
            },
        }
    }

    pub(super) fn render(
        &self,
        view: &mut SurfaceView<'_>,
        appearance: &AppearanceSnapshot,
        state: &mut UsageViewState,
    ) -> Result<(), UsageRenderError> {
        let size = view.size();
        let width = NonZeroU16::new(size.width).ok_or(UsageRenderError::WidthUnavailable)?;
        if size.height == 0 {
            return Err(UsageRenderError::HeightUnavailable);
        }

        let styles = appearance.styles();
        if view.clear(styles.overlay.styles.background) == WriteOutcome::Clipped {
            return Err(UsageRenderError::SurfaceConflict);
        }
        let glyphs = UsageGlyphs::for_appearance(styles.overlay.glyphs);

        if let Some(error) = self.error.as_ref() {
            return render_error(
                view,
                error,
                styles.overlay.styles.title,
                styles.overlay.styles.detail,
            );
        }

        let projection = self
            .projection
            .as_ref()
            .expect("a Usage error and projection cannot be absent together");
        let summary = summary_lines(
            projection,
            glyphs,
            styles.overlay.styles.title,
            styles.overlay.styles.label,
            styles.overlay.styles.detail,
        );
        let mut section_y = 0_u16;
        for (text, style) in summary {
            let height =
                paint_wrapped_text(view, Point::new(0, section_y), width.get(), &text, style)?;
            section_y = section_y.saturating_add(height);
        }
        if section_y < size.height {
            paint_rule(
                view,
                section_y,
                width.get(),
                glyphs,
                styles.overlay.styles.frame,
            )?;
            section_y = section_y.saturating_add(1);
        }

        if section_y >= size.height {
            state.reconcile(projection.receipts().len(), 1);
            return Ok(());
        }

        if width.get() >= WIDE_LAYOUT_MIN_WIDTH && size.height.saturating_sub(section_y) >= 4 {
            render_wide(
                view,
                section_y,
                projection,
                state,
                glyphs,
                styles.overlay.styles,
            )?;
        } else {
            render_stacked(
                view,
                section_y,
                projection,
                state,
                glyphs,
                styles.overlay.styles,
            )?;
        }
        Ok(())
    }
}

pub(super) const fn navigation(code: KeyCode) -> Option<UsageNavigation> {
    match code {
        KeyCode::Up => Some(UsageNavigation::Previous),
        KeyCode::Down => Some(UsageNavigation::Next),
        KeyCode::PageUp => Some(UsageNavigation::PageUp),
        KeyCode::PageDown => Some(UsageNavigation::PageDown),
        KeyCode::Home => Some(UsageNavigation::Home),
        KeyCode::End => Some(UsageNavigation::End),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct UsageGlyphs {
    selected_marker: &'static str,
    vertical: &'static str,
    horizontal: &'static str,
    separator: &'static str,
}

#[derive(Clone, Copy)]
struct UsagePalette {
    glyphs: UsageGlyphs,
    styles: crate::overlay::SelectionPanelStyles,
}

impl UsageGlyphs {
    fn for_appearance(glyphs: SelectionPanelGlyphs) -> Self {
        if glyphs == SelectionPanelGlyphs::ascii() {
            Self {
                selected_marker: ">",
                vertical: "|",
                horizontal: "-",
                separator: "/",
            }
        } else {
            Self {
                selected_marker: "›",
                vertical: "│",
                horizontal: "─",
                separator: "·",
            }
        }
    }
}

fn summary_lines(
    projection: &SessionUsageProjection,
    glyphs: UsageGlyphs,
    title: Style,
    label: Style,
    secondary: Style,
) -> Vec<(String, Style)> {
    let receipt_count = projection.receipts().len();
    if receipt_count == 0 {
        return vec![
            (
                format!(
                    "Session {} live {} receipts 0",
                    glyphs.separator, glyphs.separator
                ),
                title,
            ),
            (
                format!(
                    "Tokens {} unavailable (no completed receipts)",
                    glyphs.separator
                ),
                label,
            ),
            (
                format!(
                    "Cache read {} unavailable {} coverage=0/0",
                    glyphs.separator, glyphs.separator
                ),
                secondary,
            ),
        ];
    }

    let aggregates = projection.aggregates();
    vec![
        (
            format!(
                "Session {} live {} receipts {}",
                glyphs.separator, glyphs.separator, receipt_count
            ),
            title,
        ),
        (
            format!(
                "Tokens {} input={} {} output={} {} total={} {} reasoning={}",
                glyphs.separator,
                aggregate_text(aggregates.input_tokens(), receipt_count),
                glyphs.separator,
                aggregate_text(aggregates.output_tokens(), receipt_count),
                glyphs.separator,
                aggregate_text(aggregates.total_tokens(), receipt_count),
                glyphs.separator,
                aggregate_text(aggregates.reasoning_tokens(), receipt_count),
            ),
            label,
        ),
        (
            format!(
                "Cache read {} {}",
                glyphs.separator,
                cache_read_text(projection.cache_read()),
            ),
            secondary,
        ),
    ]
}

fn render_wide(
    view: &mut SurfaceView<'_>,
    section_y: u16,
    projection: &SessionUsageProjection,
    state: &mut UsageViewState,
    glyphs: UsageGlyphs,
    styles: crate::overlay::SelectionPanelStyles,
) -> Result<(), UsageRenderError> {
    let size = view.size();
    let section_height = size.height.saturating_sub(section_y);
    let left_width = size.width / 2;
    let divider_x = left_width;
    let right_x = divider_x.saturating_add(1);
    let right_width = size.width.saturating_sub(right_x);
    if left_width == 0 || right_width == 0 {
        return render_stacked(view, section_y, projection, state, glyphs, styles);
    }

    let viewport = usize::from(section_height.saturating_sub(1).max(1));
    state.reconcile(projection.receipts().len(), viewport);

    let left_rect = Rect::new(
        Point::new(0, section_y),
        crate::surface::Size::new(left_width, section_height),
    );
    let mut left = view
        .subview(left_rect)
        .expect("the wide Usage master pane fits inside the body");
    render_master(&mut left, projection, state, glyphs, styles)?;

    let right_rect = Rect::new(
        Point::new(right_x, section_y),
        crate::surface::Size::new(right_width, section_height),
    );
    let mut right = view
        .subview(right_rect)
        .expect("the wide Usage detail pane fits inside the body");
    render_detail(&mut right, projection, state, glyphs, styles)?;

    paint_vertical(
        view,
        divider_x,
        section_y,
        section_height,
        glyphs,
        styles.frame,
    )
}

fn render_stacked(
    view: &mut SurfaceView<'_>,
    section_y: u16,
    projection: &SessionUsageProjection,
    state: &mut UsageViewState,
    glyphs: UsageGlyphs,
    styles: crate::overlay::SelectionPanelStyles,
) -> Result<(), UsageRenderError> {
    let size = view.size();
    let section_height = size.height.saturating_sub(section_y);
    let viewport = if section_height > 4 {
        usize::from(section_height.saturating_sub(3) / 2).max(1)
    } else {
        1
    };
    state.reconcile(projection.receipts().len(), viewport);

    let list_height = 1_u16.saturating_add(u16::try_from(viewport).unwrap_or(u16::MAX));
    let list_height = list_height.min(section_height);
    let list_rect = Rect::new(
        Point::new(0, section_y),
        crate::surface::Size::new(size.width, list_height),
    );
    let mut list = view
        .subview(list_rect)
        .expect("the stacked Usage master pane fits inside the body");
    render_master(&mut list, projection, state, glyphs, styles)?;

    let divider_y = section_y.saturating_add(list_height);
    if divider_y >= size.height {
        return Ok(());
    }
    paint_rule(view, divider_y, size.width, glyphs, styles.frame)?;
    let detail_y = divider_y.saturating_add(1);
    if detail_y >= size.height {
        return Ok(());
    }
    let detail_rect = Rect::new(
        Point::new(0, detail_y),
        crate::surface::Size::new(size.width, size.height - detail_y),
    );
    let mut detail = view
        .subview(detail_rect)
        .expect("the stacked Usage detail pane fits inside the body");
    render_detail(&mut detail, projection, state, glyphs, styles)
}

fn render_master(
    view: &mut SurfaceView<'_>,
    projection: &SessionUsageProjection,
    state: &UsageViewState,
    glyphs: UsageGlyphs,
    styles: crate::overlay::SelectionPanelStyles,
) -> Result<(), UsageRenderError> {
    let size = view.size();
    paint_single_line(view, Point::new(0, 0), "Receipts", size.width, styles.title)?;
    if size.height <= 1 {
        return Ok(());
    }
    if projection.receipts().is_empty() {
        return paint_single_line(
            view,
            Point::new(0, 1),
            "No completed usage receipts.",
            size.width,
            styles.detail,
        );
    }

    for offset in 0..state_viewport(state, projection.receipts().len()) {
        let index = state.first_visible.saturating_add(offset);
        if index >= projection.receipts().len() {
            break;
        }
        let row = 1_u16.saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
        if row >= size.height {
            break;
        }
        paint_receipt_row(
            view,
            row,
            size.width,
            index,
            &projection.receipts()[index],
            index == state.selected,
            UsagePalette { glyphs, styles },
        )?;
    }
    Ok(())
}

fn state_viewport(state: &UsageViewState, receipt_count: usize) -> usize {
    receipt_count.saturating_sub(state.first_visible).max(1)
}

fn paint_receipt_row(
    view: &mut SurfaceView<'_>,
    row: u16,
    width: u16,
    index: usize,
    receipt: &SessionUsageReceipt,
    selected: bool,
    palette: UsagePalette,
) -> Result<(), UsageRenderError> {
    if width == 0 {
        return Ok(());
    }
    let UsagePalette { glyphs, styles } = palette;
    let style = if selected {
        styles.selected
    } else {
        styles.label
    };
    paint_single_line(
        view,
        Point::new(0, row),
        if selected {
            glyphs.selected_marker
        } else {
            " "
        },
        1,
        style,
    )?;
    if width >= GUTTER_WIDTH {
        paint_single_line(view, Point::new(1, row), " ", 1, style)?;
    }
    if width > GUTTER_WIDTH {
        let label = format!(
            "[{:02}] {}",
            index + 1,
            source_text(receipt, Some(glyphs.separator))
        );
        paint_single_line(
            view,
            Point::new(GUTTER_WIDTH, row),
            &label,
            width - GUTTER_WIDTH,
            style,
        )?;
    }
    Ok(())
}

fn render_detail(
    view: &mut SurfaceView<'_>,
    projection: &SessionUsageProjection,
    state: &UsageViewState,
    glyphs: UsageGlyphs,
    styles: crate::overlay::SelectionPanelStyles,
) -> Result<(), UsageRenderError> {
    let size = view.size();
    paint_single_line(view, Point::new(0, 0), "Detail", size.width, styles.title)?;
    if size.height <= 1 {
        return Ok(());
    }
    let Some(receipt) = projection.receipts().get(state.selected) else {
        return paint_single_line(
            view,
            Point::new(0, 1),
            "No receipt selected.",
            size.width,
            styles.detail,
        );
    };

    let details = detail_lines(
        receipt,
        state.selected,
        projection.receipts().len(),
        glyphs,
        styles,
    );
    let mut y = 1_u16;
    for (text, style) in details {
        if y >= size.height {
            break;
        }
        let height = paint_wrapped_text(view, Point::new(0, y), size.width, &text, style)?;
        y = y.saturating_add(height.max(1));
    }
    Ok(())
}

fn detail_lines(
    receipt: &SessionUsageReceipt,
    selected: usize,
    receipt_count: usize,
    glyphs: UsageGlyphs,
    styles: crate::overlay::SelectionPanelStyles,
) -> Vec<(String, Style)> {
    let source = source_text(receipt, Some(glyphs.separator));
    let usage = receipt.usage();
    vec![
        (
            format!("Receipt {}/{} {}", selected + 1, receipt_count, source),
            styles.selected,
        ),
        (
            format!(
                "activity={} schema={}",
                receipt.activity().activity_id().get().get(),
                receipt.schema()
            ),
            styles.detail,
        ),
        (
            format!(
                "input={} {} output={}",
                value_text(usage.input_tokens()),
                glyphs.separator,
                value_text(usage.output_tokens()),
            ),
            styles.label,
        ),
        (
            format!(
                "total={} {} reasoning={}",
                value_text(usage.total_tokens()),
                glyphs.separator,
                value_text(usage.reasoning_tokens()),
            ),
            styles.label,
        ),
        (
            format!(
                "cache_read={} {} cache_write={}",
                value_text(usage.cache_read_input_tokens()),
                glyphs.separator,
                value_text(usage.cache_write_input_tokens()),
            ),
            styles.label,
        ),
    ]
}

fn render_error(
    view: &mut SurfaceView<'_>,
    error: &SessionUsageError,
    title: Style,
    secondary: Style,
) -> Result<(), UsageRenderError> {
    let lines = [
        ("Usage error".to_owned(), title),
        (
            "Known receipt rejected; token totals unavailable.".to_owned(),
            secondary,
        ),
        (
            format!(
                "activity={} schema={}",
                error.activity().activity_id().get().get(),
                safe_text(error.schema())
            ),
            secondary,
        ),
        (format!("detail={}", safe_text(error.detail())), secondary),
    ];
    let mut y = 0_u16;
    for (text, style) in lines {
        if y >= view.size().height {
            break;
        }
        let height = paint_wrapped_text(view, Point::new(0, y), view.size().width, &text, style)?;
        y = y.saturating_add(height.max(1));
    }
    Ok(())
}

fn paint_rule(
    view: &mut SurfaceView<'_>,
    row: u16,
    width: u16,
    glyphs: UsageGlyphs,
    style: Style,
) -> Result<(), UsageRenderError> {
    if row >= view.size().height || width == 0 {
        return Ok(());
    }
    let line = glyphs.horizontal.repeat(usize::from(width));
    paint_single_line(view, Point::new(0, row), &line, width, style)
}

fn paint_vertical(
    view: &mut SurfaceView<'_>,
    column: u16,
    row: u16,
    height: u16,
    glyphs: UsageGlyphs,
    style: Style,
) -> Result<(), UsageRenderError> {
    if column >= view.size().width {
        return Ok(());
    }
    for offset in 0..height {
        let y = row.saturating_add(offset);
        if y >= view.size().height {
            break;
        }
        paint_single_line(view, Point::new(column, y), glyphs.vertical, 1, style)?;
    }
    Ok(())
}

fn paint_single_line(
    view: &mut SurfaceView<'_>,
    origin: Point,
    text: &str,
    width: u16,
    style: Style,
) -> Result<(), UsageRenderError> {
    if width == 0 || origin.y >= view.size().height {
        return Ok(());
    }
    let flow = flow_text(
        text,
        NonZeroU16::new(width).ok_or(UsageRenderError::WidthUnavailable)?,
    )
    .map_err(UsageRenderError::Text)?;
    for positioned in flow
        .glyphs
        .into_iter()
        .filter(|positioned| positioned.point.y == 0)
    {
        let x = origin
            .x
            .checked_add(positioned.point.x)
            .ok_or(UsageRenderError::SurfaceConflict)?;
        if x >= view.size().width {
            continue;
        }
        if view.write(Point::new(x, origin.y), positioned.grapheme, style) == WriteOutcome::Clipped
        {
            return Err(UsageRenderError::SurfaceConflict);
        }
    }
    Ok(())
}

fn paint_wrapped_text(
    view: &mut SurfaceView<'_>,
    origin: Point,
    width: u16,
    text: &str,
    style: Style,
) -> Result<u16, UsageRenderError> {
    if width == 0 || origin.y >= view.size().height {
        return Ok(0);
    }
    let flow = flow_text(
        text,
        NonZeroU16::new(width).ok_or(UsageRenderError::WidthUnavailable)?,
    )
    .map_err(UsageRenderError::Text)?;
    for positioned in flow.glyphs {
        let Some(y) = origin.y.checked_add(positioned.point.y) else {
            continue;
        };
        if y >= view.size().height {
            continue;
        }
        let x = origin
            .x
            .checked_add(positioned.point.x)
            .ok_or(UsageRenderError::SurfaceConflict)?;
        if x >= view.size().width {
            continue;
        }
        if view.write(Point::new(x, y), positioned.grapheme, style) == WriteOutcome::Clipped {
            return Err(UsageRenderError::SurfaceConflict);
        }
    }
    Ok(flow.height)
}
