use std::num::NonZeroU16;

use unicode_segmentation::UnicodeSegmentation;

use super::{
    EntryAvailability, PanelPaintError, SelectionEntry, SelectionPanel, SelectionPanelAppearance,
    VISIBLE_ENTRY_CAP,
};
use crate::{
    overlay::binding::{BindingHint, OverlayBindings},
    surface::{Grapheme, Point, Size, Style, SurfaceView, WriteOutcome},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedSelectionPanel {
    size: Size,
    background: Style,
    writes: Vec<PreparedWrite>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PreparedWrite {
    point: Point,
    grapheme: Grapheme,
    style: Style,
}

impl SelectionPanel {
    pub(crate) fn prepare(
        &self,
        available: Size,
        appearance: SelectionPanelAppearance,
        bindings: &OverlayBindings,
        turn_active: bool,
    ) -> Option<PreparedSelectionPanel> {
        let width = NonZeroU16::new(available.width)?;
        if width.get() < 3 || available.height < 3 {
            return None;
        }
        let visible_rows = self
            .snapshot
            .entries
            .len()
            .min(VISIBLE_ENTRY_CAP)
            .min(usize::from(available.height - 2));
        if visible_rows == 0 {
            return None;
        }
        let hints = fitting_hints(
            width,
            bindings.hints(turn_active, appearance.glyphs.rich_keys),
            &self.snapshot.title,
        )?;
        if hints.is_empty() {
            return None;
        }
        let (start, end) = self.visible_window(visible_rows);
        let height = u16::try_from(visible_rows + 2).expect("visible cap fits u16");
        let size = Size::new(width.get(), height);
        let mut prepared = PreparedSelectionPanel {
            size,
            background: appearance.styles.background,
            writes: Vec::new(),
        };
        prepared.prepare_frame(
            appearance,
            &self.snapshot.title,
            self.snapshot.title_status.as_deref(),
            &hints,
            start,
            self.snapshot.entries.len() - end,
        );
        let label_column_width = self.snapshot.entries[start..end]
            .iter()
            .filter(|entry| entry.context.is_some())
            .map(|entry| text_width(&entry.label))
            .max()
            .unwrap_or(0);
        for (row, entry) in self.snapshot.entries[start..end].iter().enumerate() {
            prepared.prepare_entry(
                u16::try_from(row + 1).expect("visible cap fits u16"),
                entry,
                self.selected.as_ref() == Some(&entry.identity),
                appearance,
                label_column_width,
            );
        }
        Some(prepared)
    }
}

impl PreparedSelectionPanel {
    pub(crate) const fn size(&self) -> Size {
        self.size
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

    fn prepare_frame(
        &mut self,
        appearance: SelectionPanelAppearance,
        title: &str,
        title_status: Option<&str>,
        hints: &[BindingHint],
        hidden_above: usize,
        hidden_below: usize,
    ) {
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
                self.push_truncated_text(
                    Point::new(
                        u16::try_from(status_start).expect("panel width fits u16"),
                        0,
                    ),
                    &format!(" · {status} "),
                    title_end,
                    styles.hint,
                );
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
        let counts = match (hidden_above, hidden_below) {
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
        let label_style = match entry.availability {
            EntryAvailability::Enabled => styles.label,
            EntryAvailability::Disabled { .. } => styles.disabled,
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
