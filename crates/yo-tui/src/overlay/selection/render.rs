use std::{num::NonZeroU16, time::Duration};

use unicode_segmentation::UnicodeSegmentation;

use super::{
    EntryAvailability, PanelPaintError, PanelTitleStatus, SelectionEntry, SelectionEntryKind,
    SelectionPanel, SelectionPanelAppearance, VISIBLE_ENTRY_CAP,
};
use crate::{
    appearance::ActivityMotionFrame,
    overlay::binding::{BindingHint, OverlayBindings},
    surface::{Grapheme, Point, Size, Style, SurfaceView, WriteOutcome},
    text::flow::{TextFlow, flow_prose},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedSelectionPanel {
    size: Size,
    background: Style,
    writes: Vec<PreparedWrite>,
    motion_period: Option<Duration>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PreparedWrite {
    point: Point,
    grapheme: Grapheme,
    style: Style,
}

struct FrameContent<'frame> {
    title: &'frame str,
    title_status: Option<&'frame PanelTitleStatus>,
    motion: ActivityMotionFrame<'frame>,
    hints: &'frame [BindingHint],
    hidden: (usize, usize),
    filter_bar: Option<&'frame super::FilterBar>,
}

impl SelectionPanel {
    #[cfg(test)]
    pub(crate) fn prepare(
        &self,
        available: Size,
        appearance: SelectionPanelAppearance,
        bindings: &OverlayBindings,
        turn_active: bool,
    ) -> Option<PreparedSelectionPanel> {
        self.prepare_with_motion(
            available,
            appearance,
            bindings,
            turn_active,
            ActivityMotionFrame::still("·"),
        )
    }

    pub(crate) fn prepare_with_motion(
        &self,
        available: Size,
        appearance: SelectionPanelAppearance,
        bindings: &OverlayBindings,
        turn_active: bool,
        motion: ActivityMotionFrame<'_>,
    ) -> Option<PreparedSelectionPanel> {
        if self.snapshot.wrapped_entries {
            return self.prepare_wrapped(available, appearance, bindings, turn_active, motion);
        }
        let width = NonZeroU16::new(available.width)?;
        if width.get() < 3 || available.height < 3 {
            return None;
        }
        let row_capacity = self
            .snapshot
            .entries
            .len()
            .min(VISIBLE_ENTRY_CAP)
            .min(usize::from(available.height - 2));
        if row_capacity == 0 {
            return None;
        }
        let hints = fitting_hints(
            width,
            bindings
                .hints(turn_active, appearance.glyphs.rich_keys)
                .into_iter()
                .filter_map(|hint| match self.snapshot.request_is_approval {
                    Some(approval) => hint.for_request(
                        approval,
                        self.snapshot.entries.iter().any(|entry| entry.is_enabled()),
                    ),
                    None => Some(hint),
                })
                .collect(),
            &self.snapshot.title,
        )?;
        if hints.is_empty() {
            return None;
        }
        let window = self.visible_window(row_capacity);
        let visible_rows = window.physical_rows();
        if visible_rows == 0 || visible_rows > row_capacity {
            return None;
        }
        let detail_rows = (usize::from(available.height) - visible_rows - 2).min(6);
        let detail = self.request_detail(width, detail_rows);
        let detail_height = detail
            .as_ref()
            .map_or(0, |flow| usize::from(flow.height).min(detail_rows));
        let height = u16::try_from(visible_rows + detail_height + 2)
            .expect("panel remains within available height");
        let size = Size::new(width.get(), height);
        let mut prepared = PreparedSelectionPanel {
            size,
            background: appearance.styles.background,
            writes: Vec::new(),
            motion_period: None,
        };
        prepared.prepare_frame(
            appearance,
            FrameContent {
                title: &self.snapshot.title,
                title_status: self.snapshot.title_status.as_ref(),
                motion,
                hints: &hints,
                hidden: (window.hidden_above, window.hidden_below),
                filter_bar: self.snapshot.filter_bar.as_ref(),
            },
        );
        let label_column_width = self.snapshot.entries[window.start..window.end]
            .iter()
            .filter(|entry| entry.context.is_some())
            .map(|entry| text_width(&entry.label))
            .max()
            .unwrap_or(0);
        let mut row = 1;
        if let Some(section) = window.pinned_section {
            prepared.prepare_entry(
                row,
                &self.snapshot.entries[section],
                false,
                appearance,
                label_column_width,
            );
            row += 1;
        }
        for entry in &self.snapshot.entries[window.start..window.end] {
            prepared.prepare_entry(
                row,
                entry,
                self.selected.as_ref() == Some(&entry.identity),
                appearance,
                label_column_width,
            );
            row += 1;
        }
        if let Some(detail) = detail {
            let clipped = usize::from(detail.height) > detail_rows;
            let body_rows = detail_height.saturating_sub(usize::from(clipped));
            for glyph in detail.glyphs {
                if usize::from(glyph.point.y) >= body_rows {
                    continue;
                }
                prepared.writes.push(PreparedWrite {
                    point: Point::new(3 + glyph.point.x, row + glyph.point.y),
                    grapheme: glyph.grapheme,
                    style: appearance.styles.detail,
                });
            }
            if clipped {
                prepared.push_truncated_text(
                    Point::new(3, row + body_rows as u16),
                    "… PgUp: full details",
                    width.get() - 1,
                    appearance.styles.hint,
                );
            }
        }
        Some(prepared)
    }

    fn prepare_wrapped(
        &self,
        available: Size,
        appearance: SelectionPanelAppearance,
        bindings: &OverlayBindings,
        turn_active: bool,
        motion: ActivityMotionFrame<'_>,
    ) -> Option<PreparedSelectionPanel> {
        let width = NonZeroU16::new(available.width)?;
        let body_width = NonZeroU16::new(available.width.checked_sub(4)?)?;
        let capacity = usize::from(available.height.checked_sub(2)?);
        if capacity == 0 {
            return None;
        }
        let flow = |index: usize| {
            let entry = &self.snapshot.entries[index];
            let mut text = entry.label.clone();
            if let Some(detail) = &entry.detail {
                text.push('\n');
                text.push_str(detail);
            }
            flow_prose(&text, body_width).ok()
        };
        let selected = self.wrapped_focus;
        let selected_flow = flow(selected)?;
        let total_lines = usize::from(selected_flow.height);
        let max_scroll = total_lines.saturating_sub(capacity);
        self.wrapped_scroll_max.set(max_scroll);
        self.wrapped_page_size
            .set(capacity.saturating_sub(1).max(1));
        let offset = self.wrapped_scroll.get().min(max_scroll);
        self.wrapped_scroll.set(offset);
        let mut height = usize::from(selected_flow.height).min(capacity);
        let mut rows = vec![(selected, selected_flow)];
        let mut start = selected;
        let mut end = selected + 1;
        while start > 0 && rows.len() < VISIBLE_ENTRY_CAP {
            let candidate = flow(start - 1)?;
            if height + usize::from(candidate.height) > capacity {
                break;
            }
            height += usize::from(candidate.height);
            start -= 1;
            rows.insert(0, (start, candidate));
        }
        while end < self.snapshot.entries.len() && rows.len() < VISIBLE_ENTRY_CAP {
            let candidate = flow(end)?;
            if height + usize::from(candidate.height) > capacity {
                break;
            }
            height += usize::from(candidate.height);
            rows.push((end, candidate));
            end += 1;
        }
        let hints = fitting_hints(
            width,
            bindings.hints(turn_active, appearance.glyphs.rich_keys),
            &self.snapshot.title,
        )?;
        let mut prepared = PreparedSelectionPanel {
            size: Size::new(width.get(), u16::try_from(height + 2).ok()?),
            background: appearance.styles.background,
            writes: Vec::new(),
            motion_period: None,
        };
        prepared.prepare_frame(
            appearance,
            FrameContent {
                title: &self.snapshot.title,
                title_status: self.snapshot.title_status.as_ref(),
                motion,
                hints: &hints,
                hidden: (start, self.snapshot.entries.len() - end),
                filter_bar: None,
            },
        );
        let mut y = 1_u16;
        for (index, flow) in rows {
            let entry = &self.snapshot.entries[index];
            let selected = self.wrapped_focus == index;
            let style = if !entry.is_enabled() {
                appearance.styles.disabled
            } else {
                appearance.styles.label
            };
            if selected {
                prepared.push_text(
                    Point::new(1, y),
                    ">",
                    width.get() - 1,
                    appearance.styles.key_hint,
                );
            }
            for glyph in flow.glyphs {
                let skip = if index == self.wrapped_focus {
                    offset
                } else {
                    0
                };
                let Some(line) = usize::from(glyph.point.y).checked_sub(skip) else {
                    continue;
                };
                let line = u16::try_from(line).ok()?;
                if usize::from(y + line) > height {
                    continue;
                }
                prepared.writes.push(PreparedWrite {
                    point: Point::new(3 + glyph.point.x, y + line),
                    grapheme: glyph.grapheme,
                    style,
                });
            }
            y = y.saturating_add(flow.height);
        }
        if max_scroll > 0 {
            let footer = prepared.size.height - 1;
            prepared.writes.retain(|write| write.point.y != footer);
            prepared.push_truncated_text(
                Point::new(1, footer),
                &format!("PgUp/Dn {}/{}", offset + 1, total_lines),
                width.get() - 1,
                appearance.styles.hint,
            );
        }
        Some(prepared)
    }

    fn request_detail(&self, width: NonZeroU16, rows: usize) -> Option<TextFlow> {
        self.snapshot.request_is_approval?;
        if rows == 0 {
            return None;
        }
        let entry = self.snapshot.entries.get(self.selected_index?)?;
        let detail = entry.detail.as_deref().filter(|text| !text.is_empty())?;
        let body_width = NonZeroU16::new(width.get().saturating_sub(4))?;
        // 화면에 들어갈 수 있는 접두부만 배치하되 잘림을 판별할 한 줄은 더 유지한다.
        let limit = usize::from(body_width.get()) * (rows + 1);
        let mut clusters = entry
            .label
            .graphemes(true)
            .chain(std::iter::once("\n"))
            .chain(detail.graphemes(true));
        let text: String = clusters.by_ref().take(limit + 1).collect();
        let mut flow = flow_prose(&text, body_width).ok()?;
        if clusters.next().is_some() {
            flow.height = flow.height.max((rows + 1) as u16);
        }
        Some(flow)
    }
}

impl PreparedSelectionPanel {
    pub(crate) const fn size(&self) -> Size {
        self.size
    }

    pub(crate) const fn motion_period(&self) -> Option<Duration> {
        self.motion_period
    }

    pub(crate) fn paint(self, view: &mut SurfaceView<'_>) -> Result<(), PanelPaintError> {
        if view.size() != self.size || view.clear(self.background) == WriteOutcome::Clipped {
            return Err(PanelPaintError::SurfaceConflict);
        }
        for write in self.writes {
            if view.write(write.point, write.grapheme, write.style) == WriteOutcome::Clipped {
                return Err(PanelPaintError::SurfaceConflict);
            }
        }
        Ok(())
    }

    fn prepare_frame(&mut self, appearance: SelectionPanelAppearance, content: FrameContent<'_>) {
        let FrameContent {
            title,
            title_status,
            motion,
            hints,
            hidden,
            filter_bar,
        } = content;
        let glyphs = appearance.glyphs;
        let styles = appearance.styles;
        let last_x = self.size.width - 1;
        let last_y = self.size.height - 1;
        for x in 0..self.size.width {
            let top = if x == 0 {
                glyphs.top_left
            } else if x == last_x {
                glyphs.top_right
            } else {
                glyphs.horizontal
            };
            let bottom = if x == 0 {
                glyphs.bottom_left
            } else if x == last_x {
                glyphs.bottom_right
            } else {
                glyphs.horizontal
            };
            self.push_glyph(Point::new(x, 0), top, styles.frame);
            self.push_glyph(Point::new(x, last_y), bottom, styles.frame);
        }
        for y in 1..last_y {
            self.push_glyph(Point::new(0, y), glyphs.vertical, styles.frame);
            self.push_glyph(Point::new(last_x, y), glyphs.vertical, styles.frame);
        }

        let content_end = last_x;
        let hint_width = hints_width(hints);
        let hint_start = usize::from(content_end)
            .checked_sub(hint_width + 1)
            .expect("header fitting reserves mandatory hint width");
        let title_end = u16::try_from(hint_start.saturating_sub(1)).expect("panel width fits u16");
        let title = format!(" {title}");
        self.push_truncated_text(
            Point::new(2.min(content_end), 0),
            &title,
            title_end,
            styles.title,
        );
        if let Some(status) = title_status {
            let status_start = 2_usize.saturating_add(text_width(&title));
            if status_start < usize::from(title_end) {
                let status_text = status.text();
                let formatted = format!(" · {status_text} ");
                self.push_truncated_text(
                    Point::new(
                        u16::try_from(status_start).expect("panel width fits u16"),
                        0,
                    ),
                    &formatted,
                    title_end,
                    styles.hint,
                );
                if matches!(status, PanelTitleStatus::Activity(_)) {
                    let available = usize::from(title_end).saturating_sub(status_start);
                    let visible_content = if text_width(&formatted) > available {
                        available.saturating_sub(text_width("…"))
                    } else {
                        available
                    };
                    let visible_status_cells = visible_content
                        .saturating_sub(text_width(" · "))
                        .min(text_width(status_text));
                    let activity_start = status_start.saturating_add(text_width(" · "));
                    self.prepare_activity_status(
                        u16::try_from(activity_start).expect("panel width fits u16"),
                        u16::try_from(activity_start.saturating_add(visible_status_cells))
                            .expect("panel width fits u16"),
                        status_text,
                        styles.activity,
                        motion,
                    );
                }
            }
        } else {
            let trailing = 2_usize.saturating_add(text_width(&title));
            if trailing < usize::from(title_end) {
                self.push_text(
                    Point::new(u16::try_from(trailing).expect("panel width fits u16"), 0),
                    " ",
                    title_end,
                    styles.title,
                );
            }
        }
        self.prepare_hints(
            u16::try_from(hint_start).expect("panel width fits u16"),
            content_end,
            hints,
            appearance,
        );
        let counts = match hidden {
            (0, 0) => String::new(),
            (above, 0) => format!(" ↑{above} "),
            (0, below) => format!(" {below}↓ "),
            (above, below) => format!(" ↑{above} · {below}↓ "),
        };
        if !counts.is_empty() {
            let start = usize::from(content_end)
                .saturating_sub(text_width(&counts) + 1)
                .max(1);
            self.push_text(
                Point::new(u16::try_from(start).expect("panel width fits u16"), last_y),
                &counts,
                content_end,
                styles.hint,
            );
        }
        if let Some(filter_bar) = filter_bar {
            self.prepare_filters(filter_bar, &counts, appearance);
        }
    }

    fn prepare_activity_status(
        &mut self,
        start: u16,
        end: u16,
        text: &str,
        activity_styles: crate::appearance::ActivityStyles,
        motion: ActivityMotionFrame<'_>,
    ) {
        let mut visible = Vec::new();
        let mut x = start;
        for text in text.graphemes(true) {
            let Ok(grapheme) = Grapheme::try_from(text) else {
                return;
            };
            let next = x.saturating_add(grapheme.width().get());
            if next > end {
                break;
            }
            visible.push((x, grapheme));
            x = next;
        }
        let Some(sheen) = motion.sheen(visible.len()) else {
            return;
        };
        for (index, (x, grapheme)) in visible.into_iter().enumerate() {
            self.writes.push(PreparedWrite {
                point: Point::new(x, 0),
                grapheme,
                style: sheen.style_at(index, activity_styles),
            });
        }
        self.motion_period = motion.period();
    }

    fn prepare_filters(
        &mut self,
        filter_bar: &super::FilterBar,
        counts: &str,
        appearance: SelectionPanelAppearance,
    ) {
        let last_y = self.size.height - 1;
        let end = self
            .size
            .width
            .saturating_sub(u16::try_from(text_width(counts)).unwrap_or(u16::MAX))
            .saturating_sub(2);
        if end <= 3 {
            return;
        }
        let (left, right) = if appearance.glyphs.rich_keys {
            ("← ", " →")
        } else {
            ("< ", " >")
        };
        let mut x = 2;
        self.push_text(Point::new(x, last_y), left, end, appearance.styles.key_hint);
        x = x.saturating_add(2);
        for (index, label) in filter_bar.labels.iter().enumerate() {
            if index > 0 {
                self.push_text(Point::new(x, last_y), " · ", end, appearance.styles.hint);
                x = x.saturating_add(3);
            }
            let style = if index == filter_bar.selected {
                appearance.styles.key_hint
            } else {
                appearance.styles.hint
            };
            self.push_truncated_text(Point::new(x, last_y), label, end, style);
            x = x.saturating_add(u16::try_from(text_width(label)).unwrap_or(u16::MAX));
            if x >= end {
                return;
            }
        }
        self.push_text(
            Point::new(x, last_y),
            right,
            end,
            appearance.styles.key_hint,
        );
    }

    fn prepare_hints(
        &mut self,
        mut x: u16,
        content_end: u16,
        hints: &[BindingHint],
        appearance: SelectionPanelAppearance,
    ) {
        for (index, hint) in hints.iter().enumerate() {
            if index > 0 {
                self.push_text(Point::new(x, 0), " · ", content_end, appearance.styles.hint);
                x = x.saturating_add(3);
            }
            let key = format!("[{}]", hint.physical());
            self.push_text(
                Point::new(x, 0),
                &key,
                content_end,
                appearance.styles.key_hint,
            );
            x = x.saturating_add(u16::try_from(text_width(&key)).unwrap_or(u16::MAX));
            self.push_text(Point::new(x, 0), " ", content_end, appearance.styles.hint);
            x = x.saturating_add(1);
            self.push_text(
                Point::new(x, 0),
                hint.caption(),
                content_end,
                appearance.styles.hint,
            );
            x = x.saturating_add(u16::try_from(text_width(hint.caption())).unwrap_or(u16::MAX));
        }
    }

    fn prepare_entry(
        &mut self,
        row: u16,
        entry: &SelectionEntry,
        selected: bool,
        appearance: SelectionPanelAppearance,
        label_column_width: usize,
    ) {
        let content_end = self.size.width - 1;
        if content_end <= 1 {
            return;
        }
        let styles = appearance.styles;
        if entry.kind == SelectionEntryKind::Section {
            self.push_truncated_text(Point::new(2, row), &entry.label, content_end, styles.label);
            return;
        }
        if entry.kind == SelectionEntryKind::Status {
            self.push_truncated_text(
                Point::new(3, row),
                &entry.label,
                content_end,
                styles.disabled,
            );
            return;
        }
        let marker = if selected {
            appearance.glyphs.selected_marker
        } else {
            " "
        };
        let marker_style = if selected {
            styles.selected
        } else {
            styles.label
        };
        self.push_text(Point::new(1, row), marker, content_end, marker_style);
        let label_style = match (selected, &entry.availability) {
            (true, EntryAvailability::Enabled) => styles.selected,
            (false, EntryAvailability::Enabled) => styles.label,
            (_, EntryAvailability::Disabled { .. }) => styles.disabled,
        };
        let label_start = 3.min(content_end);
        let available = usize::from(content_end.saturating_sub(label_start));
        if available == 0 {
            return;
        }
        let suffix = match (&entry.detail, &entry.availability) {
            (Some(detail), EntryAvailability::Enabled) => Some(detail.as_str()),
            (_, EntryAvailability::Disabled { reason }) => Some(reason.as_str()),
            (None, EntryAvailability::Enabled) => None,
        };
        let label_width = text_width(&entry.label);
        let mut primary_end = content_end;
        if let Some(suffix) = suffix {
            let suffix_width = text_width(suffix);
            if suffix_width + 2 < available {
                let suffix_start = usize::from(content_end).saturating_sub(suffix_width);
                primary_end =
                    u16::try_from(suffix_start.saturating_sub(2)).expect("panel width fits u16");
                let suffix_style = if entry.is_enabled() {
                    styles.detail
                } else {
                    styles.disabled
                };
                self.push_text(
                    Point::new(
                        u16::try_from(suffix_start).expect("panel width fits u16"),
                        row,
                    ),
                    suffix,
                    content_end,
                    suffix_style,
                );
            }
        }
        if label_width > usize::from(primary_end.saturating_sub(label_start)) {
            self.push_truncated_text(
                Point::new(label_start, row),
                &entry.label,
                primary_end,
                label_style,
            );
            return;
        }
        self.push_text(
            Point::new(label_start, row),
            &entry.label,
            primary_end,
            label_style,
        );
        if let Some(context) = &entry.context {
            let context_start = label_start
                .saturating_add(u16::try_from(label_column_width).unwrap_or(u16::MAX))
                .saturating_add(2);
            if context_start < primary_end {
                self.push_truncated_text(
                    Point::new(context_start, row),
                    context,
                    primary_end,
                    styles.detail,
                );
            }
        }
    }

    fn push_truncated_text(&mut self, point: Point, text: &str, end_x: u16, style: Style) {
        let available = usize::from(end_x.saturating_sub(point.x));
        if text_width(text) <= available {
            self.push_text(point, text, end_x, style);
            return;
        }
        if available == 0 {
            return;
        }
        let mut clipped = String::new();
        let mut used = 0;
        let ellipsis = "…";
        let ellipsis_width = text_width(ellipsis).min(available);
        for cluster in text.graphemes(true) {
            let width = grapheme_width(cluster);
            if used + width + ellipsis_width > available {
                break;
            }
            clipped.push_str(cluster);
            used += width;
        }
        if ellipsis_width <= available {
            clipped.push_str(ellipsis);
        }
        self.push_text(point, &clipped, end_x, style);
    }

    fn push_text(&mut self, point: Point, text: &str, end_x: u16, style: Style) {
        let mut x = point.x;
        for cluster in text.graphemes(true) {
            let grapheme = Grapheme::try_from(cluster)
                .expect("validated panel text contains only renderable graphemes");
            let Some(next) = x.checked_add(grapheme.width().get()) else {
                break;
            };
            if next > end_x {
                break;
            }
            self.writes.push(PreparedWrite {
                point: Point::new(x, point.y),
                grapheme,
                style,
            });
            x = next;
        }
    }

    fn push_glyph(&mut self, point: Point, text: &str, style: Style) {
        self.writes.push(PreparedWrite {
            point,
            grapheme: Grapheme::try_from(text)
                .expect("built-in panel glyphs are renderable single-cell graphemes"),
            style,
        });
    }
}

fn text_width(value: &str) -> usize {
    value.graphemes(true).map(grapheme_width).sum()
}

fn fitting_hints(
    width: NonZeroU16,
    mut hints: Vec<BindingHint>,
    title: &str,
) -> Option<Vec<BindingHint>> {
    let full_title_reserve = text_width(title).saturating_add(5);
    while !header_fits(width, &hints, full_title_reserve) {
        let Some(optional) = hints.iter().position(BindingHint::is_optional) else {
            break;
        };
        hints.remove(optional);
    }
    if header_fits(width, &hints, full_title_reserve) {
        return Some(hints);
    }
    let interrupt_is_visible = hints.iter().any(|hint| hint.caption() == "interrupt");
    let compact_title_reserve = if interrupt_is_visible { 8 } else { 5 };
    header_fits(width, &hints, compact_title_reserve).then_some(hints)
}

fn header_fits(width: NonZeroU16, hints: &[BindingHint], title_reserve: usize) -> bool {
    hints_width(hints) + title_reserve <= usize::from(width.get())
}

fn hints_width(hints: &[BindingHint]) -> usize {
    hints
        .iter()
        .map(|hint| text_width(hint.physical()) + text_width(hint.caption()) + 3)
        .sum::<usize>()
        + hints.len().saturating_sub(1) * 3
}

fn grapheme_width(value: &str) -> usize {
    usize::from(
        Grapheme::try_from(value)
            .expect("validated panel text remains renderable")
            .width()
            .get(),
    )
}
