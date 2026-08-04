//! Fitted key help and presentation-mode footer below the prompt.

use std::num::NonZeroU16;

use super::{
    ShellChromeError, ShellChromeSnapshot, ShellChromeStyles, paint_fitting_row, paint_flow,
    row_width,
};
use crate::{
    input::{
        editor::binding::NewlineBinding,
        event::{KeyCode, KeyModifiers},
        key_notation::{interrupt_notation, key_notation},
    },
    runner::PresentationMode,
    surface::{Style, SurfaceView, WriteOutcome},
    text::flow::{TextFlowError, flow_text},
};

pub(super) fn paint(
    view: &mut SurfaceView<'_>,
    snapshot: ShellChromeSnapshot<'_>,
    styles: ShellChromeStyles,
    newline_binding: NewlineBinding,
    exit_available: bool,
) -> Result<(), ShellChromeError> {
    if view.size().height == 0 {
        return Ok(());
    }
    let mode = match snapshot.mode {
        PresentationMode::Inline => "inline",
        PresentationMode::Fullscreen => "fullscreen",
    };
    let newline = key_notation(KeyCode::Enter, newline_binding.modifiers(), false);
    let exit = key_notation(KeyCode::Character('d'), KeyModifiers::CONTROL, false);
    let interrupt = interrupt_notation();
    let candidates = if snapshot.turn_active {
        let mut candidates = Vec::new();
        if exit_available {
            candidates.push(action_spans(
                &[
                    (&interrupt, "interrupt"),
                    (&newline, "newline"),
                    (&exit, "exit"),
                ],
                styles.key_hint,
                styles.mode,
            ));
        }
        candidates.push(action_spans(
            &[(&interrupt, "interrupt"), (&newline, "newline")],
            styles.key_hint,
            styles.mode,
        ));
        if exit_available {
            candidates.push(action_spans(
                &[(&interrupt, "interrupt"), (&exit, "exit")],
                styles.key_hint,
                styles.mode,
            ));
        }
        candidates.push(vec![StyledSpan::new(interrupt, styles.key_hint)]);
        candidates
    } else {
        let mut candidates = Vec::new();
        if exit_available {
            candidates.push(action_spans(
                &[(&newline, "newline"), (&exit, "exit")],
                styles.key_hint,
                styles.mode,
            ));
        }
        candidates.push(action_spans(
            &[(&newline, "newline")],
            styles.key_hint,
            styles.mode,
        ));
        if exit_available {
            candidates.push(action_spans(
                &[(&exit, "exit")],
                styles.key_hint,
                styles.mode,
            ));
        }
        candidates
    };
    paint_candidates(view, &candidates, mode, styles.mode)
}

#[derive(Clone, Debug)]
struct StyledSpan {
    text: String,
    style: Style,
}

impl StyledSpan {
    fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

fn action_spans(
    actions: &[(&str, &str)],
    key_style: Style,
    caption_style: Style,
) -> Vec<StyledSpan> {
    let mut spans = Vec::with_capacity(actions.len() * 3);
    for (index, (key, caption)) in actions.iter().enumerate() {
        if index > 0 {
            spans.push(StyledSpan::new("  ·  ", caption_style));
        }
        spans.push(StyledSpan::new(*key, key_style));
        spans.push(StyledSpan::new(format!(" {caption}"), caption_style));
    }
    spans
}

fn paint_candidates(
    view: &mut SurfaceView<'_>,
    candidates: &[Vec<StyledSpan>],
    mode: &str,
    mode_style: Style,
) -> Result<(), ShellChromeError> {
    let Some(width) = NonZeroU16::new(view.size().width) else {
        return Ok(());
    };
    let mode_width = single_row_width(mode, width).map_err(ShellChromeError::Text)?;
    let mut selected = candidates.iter().find_map(|spans| {
        let help_width = spans_width(spans, width).ok()?;
        (help_width + usize::from(help_width > 0) + mode_width <= usize::from(width.get()))
            .then_some((spans, Some(mode)))
    });
    if selected.is_none() {
        selected = candidates.iter().find_map(|spans| {
            let help_width = spans_width(spans, width).ok()?;
            (help_width <= usize::from(width.get())).then_some((spans, None))
        });
    }
    let Some((spans, visible_mode)) = selected else {
        return paint_fitting_row(view, &[mode.to_owned()], mode_style).map(|_| ());
    };
    if view.clear(mode_style) == WriteOutcome::Clipped {
        return Err(ShellChromeError::SurfaceConflict);
    }
    let mut offset = 0_u16;
    for span in spans {
        let flow = flow_text(&span.text, width).map_err(ShellChromeError::Text)?;
        let span_width = u16::try_from(row_width(&flow)).expect("a row width is bounded by u16");
        paint_flow(view, flow, offset, span.style)?;
        offset += span_width;
    }
    if let Some(mode) = visible_mode {
        let flow = flow_text(mode, width).map_err(ShellChromeError::Text)?;
        let start = usize::from(width.get()) - mode_width;
        paint_flow(
            view,
            flow,
            u16::try_from(start).expect("a row width is bounded by u16"),
            mode_style,
        )?;
    }
    Ok(())
}

fn spans_width(spans: &[StyledSpan], width: NonZeroU16) -> Result<usize, TextFlowError> {
    spans.iter().try_fold(0_usize, |total, span| {
        single_row_width(&span.text, width).map(|span_width| total + span_width)
    })
}

fn single_row_width(text: &str, width: NonZeroU16) -> Result<usize, TextFlowError> {
    let flow = flow_text(text, width)?;
    if flow.height > 1 {
        return Ok(usize::from(width.get()) + 1);
    }
    Ok(row_width(&flow))
}
