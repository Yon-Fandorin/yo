//! Ephemeral sandbox: real observations stay on the parent; synthetic input and
//! observations stay on the child. No provider dispatch escapes this boundary.
use std::time::Duration;

use yo_core::ActivityDocument;

use super::{StateEffect, StateError, TuiState};
use crate::{
    input::event::InputEvent,
    runner::{
        AgentConnection, AgentPoll, DispatchOutcome, PresentationMode, TuiDocument, TuiSessionInfo,
        preview_agent::TestAgent,
    },
};

#[derive(Debug)]
pub(super) struct Preview {
    pub(super) state: TuiState,
    agent: TestAgent,
}

impl TuiState {
    pub(super) fn open_preview(&mut self) -> Result<StateEffect, StateError> {
        if self.preview_mode {
            return Ok(StateEffect::Exit);
        }
        if self.active_turn.is_some()
            || self.starting_submission.is_some()
            || !self.pending_submissions.is_empty()
            || self.has_pending_request()
        {
            self.chat.push_notice(
                "Finish or interrupt the current turn before opening /preview.".to_owned(),
            )?;
            return Ok(StateEffect::Redraw);
        }
        let mut state = TuiState::with_session_info(TuiSessionInfo::new(
            "PREVIEW · offline test agent",
            "/preview or /exit: return to your session",
        ));
        state.preview_mode = true;
        state.prompt_templates = self.prompt_templates.clone();
        state.set_presentation_mode(PresentationMode::Fullscreen);
        state.observe_document(
            TuiDocument::new(ActivityDocument {
                title: "UI preview".to_owned(),
                markdown: r#"Isolated from your real conversation. Type a command below, or any message to chat.

## Code and documents
- `markdown`: formatted prose and code
- `syntax`: language-aware code colors
- `tables`: responsive table layout
- `diff`: highlighted code changes
- `changes`: file-change activity
- `tool-diff`: structured tool diff
- `turn-diff`: aggregate turn diff
- `links`: web links and literal fallbacks
- `file-links`: host-confirmed file links
- `footnotes`: linked notes and rich definitions
- `session-document`: host Markdown without a model turn

## Files and search
- `files-list`: files, directories and partial results
- `files-find`: file patterns and result limits
- `content-search`: matching text, context and truncation
- `file-read`: syntax-colored file output
- `files-read`: file windows and per-file errors
- `file-write`: proposed file content
- `file-edit`: proposed replacements
- `search`: web search activity

## Tools and terminal
- `tools`: simulated tool output
- `long-tools`: fold and expand tool logs
- `shell`: command and output streams
- `codex-shell`: delegated command metadata
- `shell-tail`: latest output lines
- `shell-truncated`: reported output limits and file path
- `shell-retained`: retained output in `/output`
- `terminal-wait`: background terminal wait
- `terminal-input`: literal process input
- `mcp`: named tool call and result
- `mcp-failure`: partial result and tool failure
- `mcp-image`: structured tool image output
- `resource-link`: resource identity and metadata
- `embedded-resource`: embedded code, metadata and image

## Approval and interview
- `approval`: allow or decline an action
- `approval-scopes`: session and persistent policy choices
- `approval-diff`: review the exact proposed file
- `interview`: two questions, notes and previous-answer editing

## Progress and session state
- `agent-tasks`: delegated tasks and independent child states
- `plan`: task progress checklist
- `proposed-plan`: streaming Markdown plan
- `reasoning`: public summary snapshot
- `agent-reasoning`: provider reasoning and visibility controls
- `usage`: token observation and footer
- `turn-duration`: reported turn duration
- `compaction`: context compaction progress
- `summary`: foldable compaction summary
- `branch`: foldable branch summary
- `status`, `status-update`, `status-clear`: host status changes
- `retry`: provider retry notice
- `reroute`: reported model rerouting
- `warning`: session configuration notice
- `deprecation`: migration guidance notice
- `approval-warning`: approval review notice
- `error`: failure and recovery
- `long`: streaming and scrolling

## Charts and media
- `showcase`: code, charts, images and fallbacks
- `charts`: bars, lines and steps
- `histogram`: distribution with configurable bins
- `scatter`: X/Y coordinate plot
- `chart-heights`: compact and detailed plot sizes
- `chart-series`: named series, shared axes and legend
- `diagrams`: Mermaid flow and sequence
- `message-image`: structured assistant image block
- `images`: inline PNG preview
- `image-orientation`: upright JPEG with EXIF rotation
- `image /path.png`: preview a local image
- `media-errors`: fallback states

## Navigation
Use **Alt+Up/Down** to reach item starts, **Up/Down** or **PageUp/PageDown** to scroll,
and **End** to follow the latest output. **Alt+O** folds the current activity;
**Ctrl+O** resets item choices and folds all. **Esc** interrupts.
`/preview` or `/exit` returns to your session. No model calls or file changes.
"#.to_owned(),
            }).expect("bounded preview guide").with_expanded(true),
        )?;
        self.clear_editor();
        self.preview = Some(Box::new(Preview {
            state,
            agent: TestAgent::new(),
        }));
        Ok(StateEffect::Redraw)
    }

    pub(super) fn handle_preview(
        &mut self,
        input: InputEvent,
        now: Duration,
    ) -> Result<StateEffect, StateError> {
        let preview = self
            .preview
            .as_mut()
            .expect("preview input requires a sandbox");
        match preview.state.handle(input, now)? {
            StateEffect::Exit => {
                self.preview = None;
                Ok(StateEffect::Redraw)
            },
            StateEffect::Dispatch(action) => {
                let admission = preview
                    .agent
                    .dispatch(action)
                    .map_err(|_| StateError::PreviewAgent)?;
                if let DispatchOutcome::Rejected { id, rejection } = admission {
                    preview.state.observe_submission_outcome(
                        yo_core::SubmissionOutcome::Rejected { id, rejection },
                    )?;
                }
                self.tick_preview()?;
                Ok(StateEffect::Redraw)
            },
            StateEffect::WorkspaceSearch(_)
            | StateEffect::SkillSearch(_)
            | StateEffect::PrepareImage(_) => Ok(StateEffect::Unchanged),
            other => Ok(other),
        }
    }

    #[cfg(test)]
    pub(in crate::runner) fn preview_active(&self) -> bool {
        self.preview.is_some()
    }

    pub(in crate::runner) fn preview_deadline(&self) -> Option<std::time::Instant> {
        self.preview
            .as_ref()
            .and_then(|preview| preview.agent.next_deadline())
    }

    pub(in crate::runner) fn tick_preview(&mut self) -> Result<bool, StateError> {
        let Some(preview) = self.preview.as_mut() else {
            return Ok(false);
        };
        let mut changed = false;
        if let Some(action) = preview.state.next_follow_up()? {
            let admission = preview
                .agent
                .dispatch(action)
                .map_err(|_| StateError::PreviewAgent)?;
            if let DispatchOutcome::Rejected { id, rejection } = admission {
                preview
                    .state
                    .observe_submission_outcome(yo_core::SubmissionOutcome::Rejected {
                        id,
                        rejection,
                    })?;
            }
            changed = true;
        }
        for _ in 0..32 {
            match preview.agent.poll().map_err(|_| StateError::PreviewAgent)? {
                AgentPoll::Record(record) => {
                    preview.state.observe_record(record)?;
                },
                AgentPoll::Links(_) => return Err(StateError::PreviewAgent),
                AgentPoll::StatusLine(status) => {
                    preview.state.set_status_line(status);
                },
                AgentPoll::Document(document) => {
                    preview.state.observe_document(document)?;
                },
                AgentPoll::Notice(notice) => {
                    preview.state.observe_notice(notice)?;
                },
                AgentPoll::Submission(outcome) => {
                    preview.state.observe_submission_outcome(outcome)?;
                },
                AgentPoll::Control(outcome) => {
                    preview.state.observe_control_outcome(outcome)?;
                },
                _ => break,
            }
            changed = true;
        }
        // Preview-only host-document/status commands intentionally have no Turn.
        // Their drained observation stream is their completion boundary.
        if preview.agent.next_deadline().is_none() && preview.state.active_turn.is_none() {
            preview.state.starting_submission = None;
        }
        Ok(changed)
    }
}
