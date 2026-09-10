//! Ephemeral sandbox: real observations stay on the parent; synthetic input and
//! observations stay on the child. No provider dispatch escapes this boundary.
use std::time::Duration;

use super::{StateEffect, StateError, TuiState};
use crate::{
    input::event::InputEvent,
    runner::{
        AgentConnection, AgentPoll, DispatchOutcome, PresentationMode, TuiSessionInfo,
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
        state.set_presentation_mode(PresentationMode::Fullscreen);
        state.chat.push_notice("UI preview — isolated from your real conversation.\n\nType any message to chat.\n  tools  simulated tool output\n  error  failure and recovery\n  long   streaming and scrolling\n\nEsc interrupts. /preview or /exit returns. No model calls or file changes.".to_owned())?;
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
            StateEffect::WorkspaceSearch(_) | StateEffect::SkillSearch(_) => {
                Ok(StateEffect::Unchanged)
            },
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
        for _ in 0..32 {
            match preview.agent.poll().map_err(|_| StateError::PreviewAgent)? {
                AgentPoll::Record(record) => {
                    preview.state.observe_record(record)?;
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
        Ok(changed)
    }
}
