//! Read-only Journal projections and view-local navigation for the live runner.

use std::{collections::HashMap, iter::repeat_n, num::NonZeroU16, sync::Arc, time::Duration};

use yo_core::{RequestTraceEntry, TranscriptRecord, session_repository::InheritedSessionHistory};

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
    surface::{Point, Rect, Size, Style, SurfaceView, WriteOutcome},
    text::flow::{TextFlowError, flow_text},
    transcript::{
        TranscriptItemId, TranscriptLayoutConfig, TranscriptRenderError, TranscriptScrollCommand,
        TranscriptSlice, TranscriptState, TranscriptStateError, TranscriptViewMode,
        TranscriptViewState, render_commands,
    },
};

mod changes;
mod output;
mod projection;
pub(super) use projection::format_archival_record;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ObservabilityView {
    #[default]
    Chat,
    Transcript,
    Request,
    Changes,
    Output,
}

impl ObservabilityView {
    const fn title(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Transcript => "Transcript",
            Self::Request => "Request",
            Self::Changes => "Changes",
            Self::Output => "Output",
        }
    }

    const fn short(self) -> &'static str {
        match self {
            Self::Chat => "C",
            Self::Transcript => "T",
            Self::Request => "R",
            Self::Changes => "D",
            Self::Output => "O",
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

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RequestUnavailableReason {
    NoAssociatedRequest,
    RequestAuditDetailUnavailable,
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
    expanded_tools: bool,
    focused_activity: Option<TranscriptItemId>,
    has_item_expansion: bool,
    changes: LocalTranscriptView,
    selected_change: usize,
    selected_change_key: Option<(TranscriptItemId, usize)>,
    change_count: usize,
    change_position: changes::Position,
    output: LocalTranscriptView,
    output_position: output::Position,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ObservabilityViews {
    state: ObservabilityViewState,
    pending_navigation: [Vec<TranscriptScrollCommand>; 5],
    records: Vec<TranscriptRecord>,
    request_trace: Vec<RequestTraceEntry>,
    transcript: TranscriptState,
    chat_contexts: HashMap<TranscriptItemId, usize>,
    item_expansion: Arc<HashMap<TranscriptItemId, bool>>,
    bindings: ViewSwitchBindings,
    output: output::OutputView,
    changes_view: changes::ChangesView,
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
    OutputText(TextFlowError),
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
    pub(super) fn set_item_expansion(&mut self, item: TranscriptItemId, expanded: bool) {
        if expanded == self.state.expanded_tools {
            Arc::make_mut(&mut self.item_expansion).remove(&item);
        } else {
            Arc::make_mut(&mut self.item_expansion).insert(item, expanded);
        }
        self.state.has_item_expansion = !self.item_expansion.is_empty();
    }

    pub(super) fn open_output(&mut self) {
        self.switch_to(ObservabilityView::Output);
        self.state.output_position = output::Position::latest();
        self.pending_navigation[ObservabilityView::Output as usize].clear();
    }

    pub(super) fn open_changes(&mut self) {
        self.switch_to(ObservabilityView::Changes);
        self.pending_navigation[ObservabilityView::Changes as usize]
            .push(TranscriptScrollCommand::JumpToStart);
    }

    pub(super) fn open_changes_for(&mut self, item: TranscriptItemId) {
        self.open_changes();
        self.state.selected_change_key = Some((item, 0));
    }

    pub(super) fn chat_transcript_config(
        &self,
        appearance: &AppearanceSnapshot,
    ) -> TranscriptLayoutConfig {
        appearance
            .transcript_config()
            .clone()
            .with_compact_activities(!self.state.expanded_tools)
            .with_item_expansion(Arc::clone(&self.item_expansion))
    }
    pub(super) fn observe_request_trace(&mut self, entry: RequestTraceEntry) {
        let sequence = entry.sequence();
        let index = self
            .request_trace
            .binary_search_by_key(&sequence, RequestTraceEntry::sequence)
            .unwrap_or_else(|index| index);
        if self
            .request_trace
            .get(index)
            .map(RequestTraceEntry::sequence)
            != Some(sequence)
        {
            self.request_trace.insert(index, entry);
        }
    }

    pub(super) fn wants_global_input(&self, input: &InputEvent) -> bool {
        self.bindings.target(input).is_some()
    }

    pub(super) fn observe_inherited_history(
        &mut self,
        history: &InheritedSessionHistory,
    ) -> Result<(), TranscriptStateError> {
        let mut next = u64::MAX;
        let mut append = |text: String| -> Result<(), TranscriptStateError> {
            let id = TranscriptItemId::new(next);
            next -= 1;
            self.transcript.start_assistant(id)?;
            self.transcript.append_text(id, &text)?;
            self.transcript.finalize(id)
        };
        append(super::archival::inherited_header(history))?;
        for section in history.sections() {
            append(super::archival::inherited_section_header(section))?;
            for (index, record) in section.records().iter().enumerate() {
                append(projection::format_record(index, record))?;
            }
        }
        append("Current Session · live history".to_owned())
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
        if let InputEvent::MouseScroll(lines) = input {
            if *lines == 0 {
                return ViewInputEffect::Consumed;
            }
            let scroll = if *lines < 0 {
                TranscriptScrollCommand::LineUp
            } else {
                TranscriptScrollCommand::LineDown
            };
            let Some(local) = self.active_local_mut() else {
                return ViewInputEffect::Consumed;
            };
            local.pending_scroll = Some(scroll);
            self.pending_navigation[self.state.active as usize]
                .extend(repeat_n(scroll, usize::from(lines.unsigned_abs())));
            return ViewInputEffect::Redraw;
        }
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
        if self.state.active == ObservabilityView::Chat
            && key.action == KeyAction::Press
            && key.modifiers == KeyModifiers::CONTROL
            && matches!(key.code, KeyCode::Character('o' | 'O'))
        {
            self.state.expanded_tools = !self.state.expanded_tools;
            Arc::make_mut(&mut self.item_expansion).clear();
            self.state.has_item_expansion = false;
            return ViewInputEffect::Redraw;
        }
        if self.state.active == ObservabilityView::Chat
            && key.action == KeyAction::Press
            && key.modifiers == KeyModifiers::ALT
            && matches!(key.code, KeyCode::Character('o' | 'O'))
        {
            if self.state.chat.pending_scroll.is_some() {
                return ViewInputEffect::Consumed;
            }
            let Some(item) = self.state.focused_activity else {
                return ViewInputEffect::Consumed;
            };
            let expanded = self
                .item_expansion
                .get(&item)
                .copied()
                .unwrap_or(self.state.expanded_tools);
            self.set_item_expansion(item, !expanded);
            return ViewInputEffect::Redraw;
        }
        if self.state.active == ObservabilityView::Chat
            && matches!(key.action, KeyAction::Press | KeyAction::Repeat)
            && key.modifiers == KeyModifiers::ALT
            && matches!(key.code, KeyCode::Up | KeyCode::Down)
        {
            let scroll = if key.code == KeyCode::Up {
                TranscriptScrollCommand::PreviousItem
            } else {
                TranscriptScrollCommand::NextItem
            };
            self.state.chat.pending_scroll = Some(scroll);
            self.pending_navigation[ObservabilityView::Chat as usize].push(scroll);
            return ViewInputEffect::Redraw;
        }
        if key.action == KeyAction::Release || key.modifiers != KeyModifiers::NONE {
            return if self.state.active == ObservabilityView::Chat {
                ViewInputEffect::Unhandled
            } else {
                ViewInputEffect::Consumed
            };
        }

        if self.state.active == ObservabilityView::Output
            && matches!(key.code, KeyCode::Left | KeyCode::Right)
        {
            if self.state.output_position.select(key.code == KeyCode::Left) {
                self.pending_navigation[ObservabilityView::Output as usize].clear();
            }
            return ViewInputEffect::Redraw;
        }
        if self.state.active == ObservabilityView::Changes
            && matches!(key.code, KeyCode::Left | KeyCode::Right)
        {
            let selected = if key.code == KeyCode::Left {
                self.state.selected_change.saturating_sub(1)
            } else {
                self.state
                    .selected_change
                    .saturating_add(1)
                    .min(self.state.change_count.saturating_sub(1))
            };
            if selected != self.state.selected_change {
                self.state.selected_change = selected;
                self.state.selected_change_key = None;
                self.pending_navigation[ObservabilityView::Changes as usize].clear();
                self.pending_navigation[ObservabilityView::Changes as usize]
                    .push(TranscriptScrollCommand::JumpToStart);
            }
            return ViewInputEffect::Redraw;
        }
        let Some(scroll) = navigation(key.code) else {
            return if self.state.active == ObservabilityView::Chat {
                ViewInputEffect::Unhandled
            } else {
                ViewInputEffect::Consumed
            };
        };
        let Some(local) = self.active_local_mut() else {
            return ViewInputEffect::Consumed;
        };
        local.pending_scroll = Some(scroll);
        self.pending_navigation[self.state.active as usize].push(scroll);
        ViewInputEffect::Redraw
    }

    pub(super) fn render(
        &self,
        chat: TranscriptSlice<'_>,
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
            let transcript_config = self.chat_transcript_config(appearance);
            let frame = shell::render_with_measure_hook(
                chat,
                editor,
                view,
                AgentShellRenderOptions {
                    transcript_config: &transcript_config,
                    styles: appearance.styles(),
                    scroll: &self.pending_navigation[ObservabilityView::Chat as usize],
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
            next.focused_activity = frame
                .transcript
                .and_then(|frame| frame.context_item)
                .filter(|id| {
                    chat.items()
                        .iter()
                        .any(|item| item.id() == *id && item.is_activity())
                });
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
            _ => self.active_local().and_then(|local| local.context),
        };
        if size.height == 1 {
            let status = match chrome.storage_warning {
                Some(warning) => warning.to_owned(),
                None => status_line(self.state.active, context, self.records.len(), width)?,
            };
            paint_header(view, status, width, appearance.styles().prompt.rule)?;
            after_measure();
            return Ok(ObservabilityFrame {
                cursor: Point::new(0, 0),
                state: next,
                motion_period: None,
                overlay_presented: false,
            });
        }
        let body_area = Rect::new(Point::new(0, 1), Size::new(size.width, size.height - 1));
        let mut body = view
            .subview(body_area)
            .expect("the status row leaves a body inside the complete frame");

        let mut change_section = None;
        let cursor = match self.state.active {
            ObservabilityView::Chat => unreachable!("Chat renders without a view header"),
            ObservabilityView::Transcript => {
                after_measure();
                let frame = render_commands(
                    &self.transcript,
                    &mut body,
                    appearance.transcript_config(),
                    appearance.styles().transcript,
                    &mut next.transcript.viewport,
                    &self.pending_navigation[ObservabilityView::Transcript as usize],
                )
                .map_err(ObservabilityRenderError::Transcript)?;
                next.transcript.pending_scroll = None;
                next.transcript.context = frame
                    .context_item
                    .and_then(record_index)
                    .filter(|index| *index < self.records.len());
                Point::new(0, 0)
            },
            ObservabilityView::Output => {
                after_measure();
                body.clear(appearance.styles().transcript.background);
                let columns = appearance
                    .transcript_config()
                    .max_body_width()
                    .map_or(width, |max| max.min(width));
                self.output
                    .render(
                        chat,
                        &mut body,
                        columns,
                        appearance.styles().transcript.activity.body,
                        &mut next.output_position,
                        &self.pending_navigation[ObservabilityView::Output as usize],
                    )
                    .map_err(ObservabilityRenderError::OutputText)?;
                next.output.pending_scroll = None;
                Point::new(0, 0)
            },
            ObservabilityView::Changes => {
                let sections = changes::sections(chat);
                next.change_count = sections.len();
                if let Some(index) = sections
                    .iter()
                    .position(|section| Some(section.key) == next.selected_change_key)
                {
                    next.selected_change = index;
                }
                next.selected_change = next.selected_change.min(sections.len().saturating_sub(1));
                let selected = sections.get(next.selected_change).copied();
                change_section = selected;
                next.selected_change_key = selected.map(|section| section.key);
                after_measure();
                let columns = appearance
                    .transcript_config()
                    .max_body_width()
                    .map_or(width, |max| max.min(width));
                self.changes_view
                    .render(
                        selected,
                        &mut body,
                        columns,
                        appearance.styles().transcript,
                        &mut next.change_position,
                        &self.pending_navigation[ObservabilityView::Changes as usize],
                    )
                    .map_err(ObservabilityRenderError::OutputText)?;
                next.changes.pending_scroll = None;
                Point::new(0, 0)
            },
            ObservabilityView::Request => {
                let request = self.request_projection();
                after_measure();
                render_commands(
                    &request,
                    &mut body,
                    appearance.transcript_config(),
                    appearance.styles().transcript,
                    &mut next.request.viewport,
                    &self.pending_navigation[ObservabilityView::Request as usize],
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
            ObservabilityView::Changes => next.changes.context,
            ObservabilityView::Output => next.output.context,
        };
        let status = if let Some(warning) = chrome.storage_warning {
            warning.to_owned()
        } else if next.active == ObservabilityView::Output {
            next.output_position.header(size.width)
        } else if next.active == ObservabilityView::Changes {
            changes::header(
                next.selected_change,
                next.change_count,
                size.width,
                change_section,
            )
        } else {
            status_line(next.active, context, self.records.len(), width)?
        };
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
        if self
            .active_local()
            .is_some_and(|local| local.pending_scroll.is_none())
        {
            self.pending_navigation[state.active as usize].clear();
        }
    }

    fn switch_to(&mut self, target: ObservabilityView) -> bool {
        if self.state.active == target {
            return false;
        }
        if target == ObservabilityView::Request && self.state.active != ObservabilityView::Request {
            let next_anchor = self.active_local().and_then(|local| local.context);
            if self.state.request_anchor != next_anchor {
                self.state.request.viewport = TranscriptViewState::default();
                self.state.request.pending_scroll = None;
                self.pending_navigation[ObservabilityView::Request as usize].clear();
            }
            self.state.request_anchor = next_anchor;
            self.state.request.context = next_anchor;
        }
        self.state.active = target;
        true
    }

    fn active_local(&self) -> Option<&LocalTranscriptView> {
        match self.state.active {
            ObservabilityView::Chat => Some(&self.state.chat),
            ObservabilityView::Transcript => Some(&self.state.transcript),
            ObservabilityView::Request => Some(&self.state.request),
            ObservabilityView::Changes => Some(&self.state.changes),
            ObservabilityView::Output => Some(&self.state.output),
        }
    }

    fn active_local_mut(&mut self) -> Option<&mut LocalTranscriptView> {
        match self.state.active {
            ObservabilityView::Chat => Some(&mut self.state.chat),
            ObservabilityView::Transcript => Some(&mut self.state.transcript),
            ObservabilityView::Request => Some(&mut self.state.request),
            ObservabilityView::Changes => Some(&mut self.state.changes),
            ObservabilityView::Output => Some(&mut self.state.output),
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
                &projection::request_text(
                    &self.records,
                    self.state.request_anchor,
                    &self.request_trace,
                ),
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

    pub(super) fn inline_publication_eligible(&self) -> bool {
        let pending = &self.pending_navigation[ObservabilityView::Chat as usize];
        (pending.is_empty() || pending.last() == Some(&TranscriptScrollCommand::JumpToTail))
            && self.state.inline_publication_eligible()
    }

    #[cfg(test)]
    pub(super) const fn view_positions(&self) -> (usize, usize, usize) {
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

    #[cfg(test)]
    pub(super) fn request_reason(&self) -> RequestUnavailableReason {
        projection::request_reason(&self.records, self.state.request_anchor)
    }
}

impl ObservabilityViewState {
    pub(super) const fn inline_publication_eligible(self) -> bool {
        if !matches!(self.active, ObservabilityView::Chat) || self.has_item_expansion {
            return false;
        }
        match self.chat.pending_scroll {
            Some(
                TranscriptScrollCommand::LineUp
                | TranscriptScrollCommand::PageUp
                | TranscriptScrollCommand::JumpToStart
                | TranscriptScrollCommand::PreviousItem
                | TranscriptScrollCommand::NextItem,
            ) => false,
            Some(TranscriptScrollCommand::JumpToTail) => true,
            Some(TranscriptScrollCommand::LineDown | TranscriptScrollCommand::PageDown) | None => {
                matches!(
                    self.chat_shell.transcript_mode(),
                    TranscriptViewMode::FollowTail
                )
            },
        }
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
    let area = Rect::new(Point::new(0, 0), Size::new(width.get(), 1));
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
            ObservabilityView::Changes => "Changes · F1 Chat".to_owned(),
            ObservabilityView::Output => "Output · F1 Chat".to_owned(),
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
