//! Read-only Journal projections and view-local navigation for the live runner.

use std::{collections::HashMap, num::NonZeroU16, time::Duration};

use yo_core::TranscriptRecord;

use crate::{
    appearance::AppearanceSnapshot,
    input::{
        editor::PromptEditor,
        event::{InputEvent, KeyAction, KeyCode, KeyModifiers},
        view_binding::{ViewSwitchBindings, ViewSwitchTarget},
    },
    overlay::{OverlayBindings, SelectionPanel},
    shell::{
        self, AgentShellRenderError, AgentShellRenderOptions, AgentShellViewState,
        ShellChromeSnapshot,
    },
    surface::{Point, Rect, Style, SurfaceView, WriteOutcome},
    text::flow::{TextFlowError, flow_text},
    transcript::{
        TranscriptItemId, TranscriptRenderError, TranscriptScrollCommand, TranscriptState,
        TranscriptStateError, TranscriptViewState,
    },
};

mod projection;
pub(super) use projection::format_archival_record;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ObservabilityView {
    #[default]
    Chat,
    Transcript,
    Request,
}

impl ObservabilityView {
    const fn title(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Transcript => "Transcript",
            Self::Request => "Request",
        }
    }

    const fn short(self) -> &'static str {
        match self {
            Self::Chat => "C",
            Self::Transcript => "T",
            Self::Request => "R",
        }
    }
}

impl From<ViewSwitchTarget> for ObservabilityView {
    fn from(target: ViewSwitchTarget) -> Self {
        match target {
            ViewSwitchTarget::Chat => Self::Chat,
            ViewSwitchTarget::Transcript => Self::Transcript,
            ViewSwitchTarget::Request => Self::Request,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RequestUnavailableReason {
    NoAssociatedRequest,
    RequestAuditDetailUnavailable,
}

impl RequestUnavailableReason {
    const fn code(self) -> &'static str {
        match self {
            Self::NoAssociatedRequest => "no_associated_request",
            Self::RequestAuditDetailUnavailable => "request_audit_detail_unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct LocalTranscriptView {
    viewport: TranscriptViewState,
    context: Option<usize>,
    pending_scroll: Option<TranscriptScrollCommand>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ObservabilityViewState {
    active: ObservabilityView,
    chat_shell: AgentShellViewState,
    chat: LocalTranscriptView,
    transcript: LocalTranscriptView,
    request: LocalTranscriptView,
    request_anchor: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ObservabilityViews {
    state: ObservabilityViewState,
    records: Vec<TranscriptRecord>,
    transcript: TranscriptState,
    chat_contexts: HashMap<TranscriptItemId, usize>,
    bindings: ViewSwitchBindings,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ViewInputEffect {
    Unhandled,
    Consumed,
    Redraw,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ObservabilityRenderError {
    HeaderWidthUnavailable,
    HeaderText(TextFlowError),
    HeaderSurfaceConflict,
    BodyHeightUnavailable,
    Chat(AgentShellRenderError),
    Transcript(TranscriptRenderError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ObservabilityFrame {
    pub(super) cursor: Point,
    pub(super) state: ObservabilityViewState,
    pub(super) motion_period: Option<Duration>,
    pub(super) overlay_presented: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ObservabilityRenderOptions<'frame> {
    pub(super) appearance: &'frame AppearanceSnapshot,
    pub(super) chrome: ShellChromeSnapshot<'frame>,
    pub(super) elapsed: Duration,
    pub(super) overlay: Option<&'frame SelectionPanel>,
    pub(super) overlay_bindings: &'frame OverlayBindings,
}

impl ObservabilityViews {
    pub(super) fn wants_global_input(&self, input: &InputEvent) -> bool {
        self.bindings.target(input).is_some()
    }

    pub(super) fn observe_record(
        &mut self,
        record: &TranscriptRecord,
        changed_chat_item: Option<TranscriptItemId>,
    ) -> Result<(), TranscriptStateError> {
        let index = self.records.len();
        let id = TranscriptItemId::new(
            u64::try_from(index)
                .ok()
                .and_then(|value| value.checked_add(1))
                .expect("a Journal projection cannot exceed addressable memory"),
        );
        self.transcript.start_assistant(id)?;
        self.transcript
            .append_text(id, &projection::format_record(index, record))?;
        self.transcript.finalize(id)?;
        self.records.push(record.clone());
        if let Some(item) = changed_chat_item {
            self.chat_contexts.insert(item, index);
        }
        Ok(())
    }

    pub(super) fn handle_global(&mut self, input: &InputEvent) -> ViewInputEffect {
        if let Some(target) = self.bindings.target(input) {
            return if self.switch_to(target.into()) {
                ViewInputEffect::Redraw
            } else {
                ViewInputEffect::Consumed
            };
        }

        ViewInputEffect::Unhandled
    }

    pub(super) fn handle_local(&mut self, input: &InputEvent) -> ViewInputEffect {
        if input.is_interrupt_key() {
            return ViewInputEffect::Unhandled;
        }

        let InputEvent::Key(key) = input else {
            return if self.state.active == ObservabilityView::Chat {
                ViewInputEffect::Unhandled
            } else {
                ViewInputEffect::Consumed
            };
        };
        if key.action == KeyAction::Release || key.modifiers != KeyModifiers::NONE {
            return if self.state.active == ObservabilityView::Chat {
                ViewInputEffect::Unhandled
            } else {
                ViewInputEffect::Consumed
            };
        }

        let Some(scroll) = navigation(key.code) else {
            return if self.state.active == ObservabilityView::Chat {
                ViewInputEffect::Unhandled
            } else {
                ViewInputEffect::Consumed
            };
        };
        let local = self.active_local_mut();
        local.pending_scroll = Some(scroll);
        ViewInputEffect::Redraw
    }

    pub(super) fn render(
        &self,
        chat: &TranscriptState,
        editor: &PromptEditor,
        view: &mut SurfaceView<'_>,
        options: ObservabilityRenderOptions<'_>,
        after_measure: impl FnOnce(),
    ) -> Result<ObservabilityFrame, ObservabilityRenderError> {
        let ObservabilityRenderOptions {
            appearance,
            chrome,
            elapsed,
            overlay,
            overlay_bindings,
        } = options;
        let size = view.size();
        let width =
            NonZeroU16::new(size.width).ok_or(ObservabilityRenderError::HeaderWidthUnavailable)?;
        if size.height == 0 {
            return Err(ObservabilityRenderError::BodyHeightUnavailable);
        }
        let mut next = self.state;
        if self.state.active == ObservabilityView::Chat {
            let frame = shell::render_with_measure_hook(
                chat,
                editor,
                view,
                AgentShellRenderOptions {
                    transcript_config: appearance.transcript_config(),
                    styles: appearance.styles(),
                    scroll: self.state.chat.pending_scroll,
                    frame_prompt: size.height >= shell::MIN_FRAMED_PROMPT_HEIGHT,
                    chrome,
                    activity_motion: appearance.activity_motion_frame(elapsed),
                    overlay,
                    overlay_bindings,
                },
                &mut next.chat_shell,
                after_measure,
            )
            .map_err(ObservabilityRenderError::Chat)?;
            next.chat.pending_scroll = None;
            next.chat.context = frame
                .transcript
                .and_then(|transcript| transcript.context_item)
                .and_then(|item| self.chat_contexts.get(&item).copied());
            return Ok(ObservabilityFrame {
                cursor: frame.cursor,
                state: next,
                motion_period: frame.motion_period,
                overlay_presented: frame.overlay_area.is_some(),
            });
        }
        let context = match self.state.active {
            ObservabilityView::Request => self.state.request_anchor,
            _ => self.active_local().context,
        };
        if size.height == 1 {
            let status = status_line(self.state.active, context, self.records.len(), width)?;
            paint_header(view, status, width, appearance.styles().prompt.rule)?;
            after_measure();
            return Ok(ObservabilityFrame {
                cursor: Point::new(0, 0),
                state: next,
                motion_period: None,
                overlay_presented: false,
            });
        }
        let body_area = Rect::new(
            Point::new(0, 1),
            crate::surface::Size::new(size.width, size.height - 1),
        );
        let mut body = view
            .subview(body_area)
            .expect("the status row leaves a body inside the complete frame");

        let cursor = match self.state.active {
            ObservabilityView::Chat => unreachable!("Chat renders without a view header"),
            ObservabilityView::Transcript => {
                after_measure();
                let frame = crate::transcript::render(
                    &self.transcript,
                    &mut body,
                    appearance.transcript_config(),
                    appearance.styles().transcript,
                    &mut next.transcript.viewport,
                    self.state.transcript.pending_scroll,
                )
                .map_err(ObservabilityRenderError::Transcript)?;
                next.transcript.pending_scroll = None;
                next.transcript.context = frame.context_item.and_then(record_index);
                Point::new(0, 0)
            },
            ObservabilityView::Request => {
                let request = self.request_projection();
                after_measure();
                crate::transcript::render(
                    &request,
                    &mut body,
                    appearance.transcript_config(),
                    appearance.styles().transcript,
                    &mut next.request.viewport,
                    self.state.request.pending_scroll,
                )
                .map_err(ObservabilityRenderError::Transcript)?;
                next.request.pending_scroll = None;
                Point::new(0, 0)
            },
        };
        let context = match next.active {
            ObservabilityView::Request => next.request_anchor,
            ObservabilityView::Chat => next.chat.context,
            ObservabilityView::Transcript => next.transcript.context,
        };
        let status = status_line(next.active, context, self.records.len(), width)?;
        paint_header(view, status, width, appearance.styles().prompt.rule)?;
        Ok(ObservabilityFrame {
            cursor,
            state: next,
            motion_period: None,
            overlay_presented: false,
        })
    }

    pub(super) fn commit(&mut self, state: ObservabilityViewState) {
        self.state = state;
    }

    fn switch_to(&mut self, target: ObservabilityView) -> bool {
        if self.state.active == target {
            return false;
        }
        if target == ObservabilityView::Request && self.state.active != ObservabilityView::Request {
            let next_anchor = self.active_local().context;
            if self.state.request_anchor != next_anchor {
                self.state.request.viewport = TranscriptViewState::default();
                self.state.request.pending_scroll = None;
            }
            self.state.request_anchor = next_anchor;
            self.state.request.context = next_anchor;
        }
        self.state.active = target;
        true
    }

    fn active_local(&self) -> &LocalTranscriptView {
        match self.state.active {
            ObservabilityView::Chat => &self.state.chat,
            ObservabilityView::Transcript => &self.state.transcript,
            ObservabilityView::Request => &self.state.request,
        }
    }

    fn active_local_mut(&mut self) -> &mut LocalTranscriptView {
        match self.state.active {
            ObservabilityView::Chat => &mut self.state.chat,
            ObservabilityView::Transcript => &mut self.state.transcript,
            ObservabilityView::Request => &mut self.state.request,
        }
    }

    fn request_projection(&self) -> TranscriptState {
        let mut projection = TranscriptState::new();
        let id = TranscriptItemId::new(1);
        projection
            .start_assistant(id)
            .expect("a fresh Request projection has no duplicate IDs");
        projection
            .append_text(
                id,
                &projection::request_text(&self.records, self.state.request_anchor),
            )
            .expect("the fresh Request item is streaming");
        projection
            .finalize(id)
            .expect("the fresh Request item is finalized exactly once");
        projection
    }

    pub(super) const fn active(&self) -> ObservabilityView {
        self.state.active
    }

    #[cfg(test)]
    pub(super) fn request_reason(&self) -> RequestUnavailableReason {
        projection::request_reason(&self.records, self.state.request_anchor)
    }

    #[cfg(test)]
    pub(super) const fn view_positions(&self) -> (u16, u16, u16) {
        (
            self.state.chat_shell.transcript_first_visible_row(),
            self.state.transcript.viewport.first_visible_row(),
            self.state.request.viewport.first_visible_row(),
        )
    }

    #[cfg(test)]
    pub(super) const fn chat_has_pending_scroll(&self) -> bool {
        self.state.chat.pending_scroll.is_some()
    }

    #[cfg(test)]
    pub(super) fn chat_context_count(&self) -> usize {
        self.chat_contexts.len()
    }
}

fn navigation(code: KeyCode) -> Option<TranscriptScrollCommand> {
    match code {
        KeyCode::Up => Some(TranscriptScrollCommand::LineUp),
        KeyCode::Down => Some(TranscriptScrollCommand::LineDown),
        KeyCode::PageUp => Some(TranscriptScrollCommand::PageUp),
        KeyCode::PageDown => Some(TranscriptScrollCommand::PageDown),
        KeyCode::Home => Some(TranscriptScrollCommand::JumpToStart),
        KeyCode::End => Some(TranscriptScrollCommand::JumpToTail),
        _ => None,
    }
}

fn record_index(item: TranscriptItemId) -> Option<usize> {
    usize::try_from(item.get()).ok()?.checked_sub(1)
}

fn paint_header(
    view: &mut SurfaceView<'_>,
    text: String,
    width: NonZeroU16,
    style: Style,
) -> Result<(), ObservabilityRenderError> {
    let area = Rect::new(Point::new(0, 0), crate::surface::Size::new(width.get(), 1));
    let mut header = view
        .subview(area)
        .expect("the header is inside the complete frame");
    if header.clear(style) == WriteOutcome::Clipped {
        return Err(ObservabilityRenderError::HeaderSurfaceConflict);
    }
    let flow = flow_text(&text, width).map_err(ObservabilityRenderError::HeaderText)?;
    for positioned in flow
        .glyphs
        .into_iter()
        .filter(|positioned| positioned.point.y == 0)
    {
        if header.write(positioned.point, positioned.grapheme, style) == WriteOutcome::Clipped {
            return Err(ObservabilityRenderError::HeaderSurfaceConflict);
        }
    }
    Ok(())
}

fn status_line(
    active: ObservabilityView,
    context: Option<usize>,
    record_count: usize,
    width: NonZeroU16,
) -> Result<String, ObservabilityRenderError> {
    let context = context.map_or_else(
        || "-".to_owned(),
        |index| format!("{}/{}", index + 1, record_count),
    );
    let compact = active.short();
    let candidates = [
        format!(
            "{} · context {} · F1 Chat · F2 Transcript · F3 Request",
            active.title(),
            context
        ),
        format!("{} · F1 Chat · F2 Transcript · F3 Request", active.title()),
        format!("{} · F1/F2/F3", active.title()),
        match active {
            ObservabilityView::Chat => "[C]123".to_owned(),
            ObservabilityView::Transcript => "[T]123".to_owned(),
            ObservabilityView::Request => "[R]123".to_owned(),
        },
        format!("{compact}123"),
        format!("[{compact}]"),
        compact.to_owned(),
    ];
    for candidate in candidates {
        let measured =
            flow_text(&candidate, width).map_err(ObservabilityRenderError::HeaderText)?;
        if measured.height <= 1 {
            return Ok(candidate);
        }
    }
    unreachable!("a nonzero-width status row always fits the one-cell mode identifier")
}
