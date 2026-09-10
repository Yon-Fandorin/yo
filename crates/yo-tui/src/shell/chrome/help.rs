//! Fitted key help and presentation-mode footer below the prompt.

use std::num::NonZeroU16;

use super::{
    RequestPrompt, ShellChromeError, ShellChromeSnapshot, ShellChromeStyles, paint_fitting_row,
    paint_flow, row_width,
};
use crate::{
    input::{
        editor::binding::NewlineBinding,
        event::{KeyCode, KeyModifiers},
        key_notation::{interrupt_notation, key_notation},
    },
    overlay::OverlayBindings,
    runner::PresentationMode,
    surface::{Style, SurfaceView, WriteOutcome},
    text::flow::{TextFlowError, flow_text},
};

pub(in crate::shell) fn paint_request(
    view: &mut SurfaceView<'_>,
    request: RequestPrompt,
    styles: ShellChromeStyles,
    newline_binding: NewlineBinding,
) -> Result<(), ShellChromeError> {
    let newline = key_notation(KeyCode::Enter, newline_binding.modifiers(), false);
    let (primary, secondary) = match request {
        RequestPrompt::Approval => (("Enter", "confirm"), ("Esc", "decline")),
        RequestPrompt::Answer | RequestPrompt::Choice => (("Enter", "answer"), ("Esc", "cancel")),
        RequestPrompt::Notes => (("Enter", "send both"), ("Esc", "cancel")),
    };
    let extra = match request {
        RequestPrompt::Approval => ("Up/Down", "choose"),
        RequestPrompt::Answer => (newline.as_str(), "newline"),
        RequestPrompt::Choice => ("Tab", "add notes"),
        RequestPrompt::Notes => ("Tab", "choices"),
    };
    let candidates = [
        action_spans(&[primary, extra, secondary], styles.key_hint, styles.mode),
        action_spans(&[primary, secondary], styles.key_hint, styles.mode),
        action_spans(&[primary], styles.key_hint, styles.mode),
    ];
    paint_candidates(view, &candidates, "", styles.mode)
}

pub(in crate::shell) fn paint_overlay(
    view: &mut SurfaceView<'_>,
    bindings: &OverlayBindings,
    active: bool,
    styles: ShellChromeStyles,
) -> Result<(), ShellChromeError> {
    let hints = bindings.hints(active, false);
    let actions = hints
        .iter()
        .map(|hint| (hint.physical(), hint.caption()))
        .collect::<Vec<_>>();
    let mut candidates = vec![action_spans(&actions, styles.key_hint, styles.mode)];
    // The panel header retains close/interrupt; the footer prioritizes acceptance.
    let essential = actions
        .iter()
        .copied()
        .filter(|(_, caption)| *caption != "move")
        .collect::<Vec<_>>();
    candidates.push(action_spans(&essential, styles.key_hint, styles.mode));
    for action in essential {
        candidates.push(action_spans(&[action], styles.key_hint, styles.mode));
    }
    paint_candidates(view, &candidates, "", styles.mode)
}

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
    let mut candidates = if snapshot.turn_active {
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
        let mut primary = vec![("Enter", "send"), (newline.as_str(), "newline")];
        if exit_available {
            primary.push((&exit, "exit"));
        }
        let mut discoverable = primary.clone();
        discoverable.insert(2, ("@", "files"));
        candidates.push(action_spans(&discoverable, styles.key_hint, styles.mode));
        candidates.push(action_spans(&primary, styles.key_hint, styles.mode));
        candidates.push(action_spans(
            &[("Enter", "send"), (&newline, "newline")],
            styles.key_hint,
            styles.mode,
        ));
        candidates.push(action_spans(
            &[("Enter", "send")],
            styles.key_hint,
            styles.mode,
        ));
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
    let queue_key = key_notation(
        if newline_binding.matches(KeyModifiers::ALT) {
            KeyCode::Character('q')
        } else {
            KeyCode::Enter
        },
        KeyModifiers::ALT,
        false,
    );
    let recall_key = key_notation(KeyCode::Character('r'), KeyModifiers::ALT, false);
    if snapshot.queued_messages > 0 {
        let queue = format!(
            "Queued {}{}",
            snapshot.queued_messages,
            if snapshot.queue_paused { " paused" } else { "" }
        );
        let queue_help = action_spans(
            &[
                (queue.as_str(), ""),
                (recall_key.as_str(), "edit/pause"),
                (queue_key.as_str(), "queue/resume"),
            ],
            styles.key_hint,
            styles.mode,
        );
        candidates.insert(0, queue_help);
        candidates.insert(1, vec![StyledSpan::new(queue, styles.key_hint)]);
        candidates.insert(
            2,
            vec![StyledSpan::new(
                format!(
                    "Q:{}{}",
                    snapshot.queued_messages,
                    if snapshot.queue_paused { "!" } else { "" }
                ),
                styles.key_hint,
            )],
        );
    } else if snapshot.turn_active {
        candidates.insert(
            0,
            action_spans(
                &[
                    (queue_key.as_str(), "queue"),
                    (&interrupt_notation(), "interrupt"),
                ],
                styles.key_hint,
                styles.mode,
            ),
        );
    }
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
    // Preserve the most useful actions first; the mode label uses spare space.
    let selected = candidates.iter().find_map(|spans| {
        let help_width = spans_width(spans, width).ok()?;
        (help_width <= usize::from(width.get())).then_some((
            spans,
            (help_width + usize::from(help_width > 0) + mode_width <= usize::from(width.get()))
                .then_some(mode),
        ))
    });
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
