//! Static rows surrounding the prompt inside the agent shell.

use std::{num::NonZeroU16, ops::Range, time::Duration};

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    appearance::{ActivityMotionFrame, ActivityStyles},
    input::{editor::binding::NewlineBinding, key_notation::interrupt_notation},
    runner::PresentationMode,
    surface::{Point, Rect, Size, Style, SurfaceView, WriteOutcome},
    text::flow::{TextFlowError, flow_text},
};

mod help;

const WORKING_LABEL: &str = "Working";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ShellChromeSnapshot<'value> {
    pub(crate) turn_active: bool,
    pub(crate) backend: Option<&'value str>,
    pub(crate) workspace: &'value str,
    pub(crate) mode: PresentationMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ShellChromeStyles {
    pub(crate) activity: ActivityStyles,
    pub(crate) metrics: Style,
    pub(crate) mode: Style,
    pub(crate) key_hint: Style,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ShellChromeLayout {
    pub(super) transcript: Rect,
    pub(super) transient: Rect,
    pub(super) prompt: Rect,
    pub(super) metrics: Rect,
    pub(super) mode: Rect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShellChromeError {
    Text(TextFlowError),
    SurfaceConflict,
}

pub(super) fn layout(
    area: Rect,
    prompt_desired: NonZeroU16,
    _turn_active: bool,
) -> ShellChromeLayout {
    let mut remaining = area.size.height;
    let mut activity_rows = 0;
    let mut metrics_rows = 0;
    let mut mode_rows = 0;
    let mut prompt_rows = u16::from(remaining > 0);
    remaining = remaining.saturating_sub(prompt_rows);

    // Tiny Chat surfaces retain two readable transcript rows before optional chrome detail.
    // Footer detail is useful, but it must not turn the application into a prompt-only surface.
    let transcript_floor = remaining.min(2);
    remaining -= transcript_floor;

    if remaining > 0 {
        activity_rows = 1;
        remaining -= 1;
    }

    if remaining > 0 {
        metrics_rows = 1;
        remaining -= 1;
    }
    if remaining > 0 {
        mode_rows = 1;
        remaining -= 1;
    }

    let prompt_extra = prompt_desired
        .get()
        .saturating_sub(prompt_rows)
        .min(remaining);
    prompt_rows += prompt_extra;
    remaining -= prompt_extra;

    let separator_rows = u16::from(remaining > 0);
    remaining = remaining.saturating_sub(separator_rows);
    let transcript_height = transcript_floor + remaining;
    let chrome_origin = area.origin.y + transcript_height;
    let transient_rows = separator_rows + activity_rows;
    let transient_origin = chrome_origin;
    let prompt_origin = transient_origin + transient_rows;
    let metrics_origin = prompt_origin + prompt_rows;
    let mode_origin = metrics_origin + metrics_rows;

    ShellChromeLayout {
        transcript: Rect::new(area.origin, Size::new(area.size.width, transcript_height)),
        transient: Rect::new(
            Point::new(area.origin.x, transient_origin),
            Size::new(area.size.width, transient_rows),
        ),
        prompt: Rect::new(
            Point::new(area.origin.x, prompt_origin),
            Size::new(area.size.width, prompt_rows),
        ),
        metrics: Rect::new(
            Point::new(area.origin.x, metrics_origin),
            Size::new(area.size.width, metrics_rows),
        ),
        mode: Rect::new(
            Point::new(area.origin.x, mode_origin),
            Size::new(area.size.width, mode_rows),
        ),
    }
}

pub(super) fn paint_transient(
    view: &mut SurfaceView<'_>,
    snapshot: ShellChromeSnapshot<'_>,
    styles: ShellChromeStyles,
    motion: ActivityMotionFrame<'_>,
    show_shortcuts: bool,
) -> Result<Option<Duration>, ShellChromeError> {
    if view.size().height == 0 || !snapshot.turn_active {
        return Ok(None);
    }
    let marker = motion.marker();
    let marker_graphemes = marker.graphemes(true).count();
    let trailing_cells = motion
        .reserved_marker_width()
        .saturating_sub(motion.marker_width());
    let marker_region = format!("{marker}{}", " ".repeat(usize::from(trailing_cells)));
    let marker_region_graphemes = marker_graphemes + usize::from(trailing_cells);
    let marker_padding = marker_graphemes..marker_region_graphemes;
    let interrupt = interrupt_notation();
    let working_graphemes = WORKING_LABEL.graphemes(true).count();
    let candidates = if show_shortcuts {
        vec![
            ActivityRowCandidate::new(
                format!("{marker_region} {WORKING_LABEL}  ·  {interrupt} interrupt"),
                Some(0..marker_graphemes),
                Some(marker_padding.clone()),
                Some((marker_region_graphemes + 1, working_graphemes)),
            ),
            ActivityRowCandidate::new(
                format!("{marker_region} {interrupt}"),
                Some(0..marker_graphemes),
                Some(marker_padding.clone()),
                None,
            ),
            ActivityRowCandidate::new(interrupt, None, None, None),
            ActivityRowCandidate::new(
                marker_region,
                Some(0..marker_graphemes),
                Some(marker_padding),
                None,
            ),
        ]
    } else {
        vec![
            ActivityRowCandidate::new(
                format!("{marker_region} {WORKING_LABEL}"),
                Some(0..marker_graphemes),
                Some(marker_padding),
                Some((marker_region_graphemes + 1, working_graphemes)),
            ),
            ActivityRowCandidate::new(WORKING_LABEL, None, None, Some((0, working_graphemes))),
        ]
    };
    let row = view.size().height - 1;
    let mut content = view
        .subview(Rect::new(
            Point::new(0, row),
            Size::new(view.size().width, 1),
        ))
        .expect("the transient content row is inside its reserved area");
    paint_fitting_activity_row(&mut content, &candidates, styles, motion)
}

pub(super) fn paint_metrics(
    view: &mut SurfaceView<'_>,
    snapshot: ShellChromeSnapshot<'_>,
    style: Style,
) -> Result<(), ShellChromeError> {
    if view.size().height == 0 {
        return Ok(());
    }
    let mut groups = StatusGroups::default();
    if let Some(backend) = snapshot.backend {
        groups.left.push(StatusSegment::new(backend, 100));
    }
    if !snapshot.workspace.is_empty() {
        groups.left.push(StatusSegment::new(snapshot.workspace, 30));
    }
    paint_status_groups(view, groups, style)
}

pub(super) fn paint_mode(
    view: &mut SurfaceView<'_>,
    snapshot: ShellChromeSnapshot<'_>,
    styles: ShellChromeStyles,
    newline_binding: NewlineBinding,
    exit_available: bool,
) -> Result<(), ShellChromeError> {
    help::paint(view, snapshot, styles, newline_binding, exit_available)
}

fn paint_fitting_row(
    view: &mut SurfaceView<'_>,
    candidates: &[String],
    style: Style,
) -> Result<Option<usize>, ShellChromeError> {
    let Some(width) = NonZeroU16::new(view.size().width) else {
        return Ok(None);
    };
    let mut selected = None;
    for (index, candidate) in candidates.iter().enumerate() {
        let flow = flow_text(candidate, width).map_err(ShellChromeError::Text)?;
        if flow.height <= 1 {
            selected = Some((index, flow));
            break;
        }
    }
    let Some((index, flow)) = selected else {
        return Ok(None);
    };
    if view.clear(style) == WriteOutcome::Clipped {
        return Err(ShellChromeError::SurfaceConflict);
    }
    for positioned in flow.glyphs {
        if view.write(positioned.point, positioned.grapheme, style) == WriteOutcome::Clipped {
            return Err(ShellChromeError::SurfaceConflict);
        }
    }
    Ok(Some(index))
}

fn paint_fitting_activity_row(
    view: &mut SurfaceView<'_>,
    candidates: &[ActivityRowCandidate],
    styles: ShellChromeStyles,
    motion: ActivityMotionFrame<'_>,
) -> Result<Option<Duration>, ShellChromeError> {
    let Some(width) = NonZeroU16::new(view.size().width) else {
        return Ok(None);
    };
    let mut selected = None;
    for (index, candidate) in candidates.iter().enumerate() {
        let flow = flow_text(&candidate.text, width).map_err(ShellChromeError::Text)?;
        if flow.height <= 1 {
            selected = Some((index, flow));
            break;
        }
    }
    let Some((index, flow)) = selected else {
        return Ok(None);
    };
    let candidate = &candidates[index];
    let sheen = candidate.working.and_then(|(_, count)| motion.sheen(count));
    let base = motion.static_style(styles.activity);
    if view.clear(base) == WriteOutcome::Clipped {
        return Err(ShellChromeError::SurfaceConflict);
    }
    for (grapheme_index, positioned) in flow.glyphs.into_iter().enumerate() {
        if candidate
            .blank
            .as_ref()
            .is_some_and(|blank| blank.contains(&grapheme_index))
        {
            continue;
        }
        let style = if candidate
            .marker
            .as_ref()
            .is_some_and(|marker| marker.contains(&grapheme_index))
        {
            motion.marker_style(styles.activity)
        } else if let (Some((start, count)), Some(sheen)) = (candidate.working, sheen) {
            let end = start.saturating_add(count);
            if (start..end).contains(&grapheme_index) {
                sheen.style_at(grapheme_index - start, styles.activity)
            } else {
                base
            }
        } else {
            base
        };
        if view.write(positioned.point, positioned.grapheme, style) == WriteOutcome::Clipped {
            return Err(ShellChromeError::SurfaceConflict);
        }
    }
    Ok((candidate.marker.is_some() || sheen.is_some())
        .then(|| motion.period())
        .flatten())
}

struct ActivityRowCandidate {
    text: String,
    marker: Option<Range<usize>>,
    blank: Option<Range<usize>>,
    working: Option<(usize, usize)>,
}

impl ActivityRowCandidate {
    fn new(
        text: impl Into<String>,
        marker: Option<Range<usize>>,
        blank: Option<Range<usize>>,
        working: Option<(usize, usize)>,
    ) -> Self {
        Self {
            text: text.into(),
            marker,
            blank,
            working,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct StatusGroups {
    left: Vec<StatusSegment>,
    right: Vec<StatusSegment>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StatusSegment {
    text: String,
    priority: u8,
}

impl StatusSegment {
    fn new(text: impl Into<String>, priority: u8) -> Self {
        Self {
            text: text.into(),
            priority,
        }
    }
}

fn paint_status_groups(
    view: &mut SurfaceView<'_>,
    mut groups: StatusGroups,
    style: Style,
) -> Result<(), ShellChromeError> {
    let Some(width) = NonZeroU16::new(view.size().width) else {
        return Ok(());
    };
    retain_renderable_segments(&mut groups, width);
    loop {
        let left = join_segments(&groups.left);
        let right = join_segments(&groups.right);
        let (Ok(left_flow), Ok(right_flow)) = (flow_text(&left, width), flow_text(&right, width))
        else {
            if drop_lowest_priority(&mut groups) {
                continue;
            }
            return Ok(());
        };
        let left_width = row_width(&left_flow);
        let right_width = row_width(&right_flow);
        let gap = usize::from(!left.is_empty() && !right.is_empty());
        if left_flow.height <= 1
            && right_flow.height <= 1
            && left_width + gap + right_width <= usize::from(width.get())
        {
            if view.clear(style) == WriteOutcome::Clipped {
                return Err(ShellChromeError::SurfaceConflict);
            }
            paint_flow(view, left_flow, 0, style)?;
            let right_start = usize::from(width.get()).saturating_sub(right_width);
            paint_flow(
                view,
                right_flow,
                u16::try_from(right_start).expect("status width is bounded by u16"),
                style,
            )?;
            return Ok(());
        }
        if !drop_lowest_priority(&mut groups) {
            return Ok(());
        }
    }
}

fn retain_renderable_segments(groups: &mut StatusGroups, width: NonZeroU16) {
    let fits = |segment: &StatusSegment| {
        flow_text(&segment.text, width).is_ok_and(|flow| flow.height <= 1)
    };
    groups.left.retain(&fits);
    groups.right.retain(fits);
}

fn join_segments(segments: &[StatusSegment]) -> String {
    segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join(" · ")
}

fn row_width(flow: &crate::text::flow::TextFlow) -> usize {
    flow.glyphs
        .iter()
        .map(|positioned| usize::from(positioned.point.x + positioned.grapheme.width().get()))
        .max()
        .unwrap_or(0)
}

fn paint_flow(
    view: &mut SurfaceView<'_>,
    flow: crate::text::flow::TextFlow,
    offset: u16,
    style: Style,
) -> Result<(), ShellChromeError> {
    for positioned in flow.glyphs {
        let point = Point::new(positioned.point.x + offset, positioned.point.y);
        if view.write(point, positioned.grapheme, style) == WriteOutcome::Clipped {
            return Err(ShellChromeError::SurfaceConflict);
        }
    }
    Ok(())
}

fn drop_lowest_priority(groups: &mut StatusGroups) -> bool {
    let lowest = groups
        .left
        .iter()
        .chain(&groups.right)
        .map(|segment| segment.priority)
        .min();
    let Some(lowest) = lowest else {
        return false;
    };
    if let Some(index) = groups
        .right
        .iter()
        .rposition(|segment| segment.priority == lowest)
    {
        groups.right.remove(index);
        return true;
    }
    let index = groups
        .left
        .iter()
        .rposition(|segment| segment.priority == lowest)
        .expect("the minimum priority came from one status group");
    groups.left.remove(index);
    true
}

#[cfg(test)]
mod tests;
