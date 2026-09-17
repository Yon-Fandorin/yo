use std::{
    collections::VecDeque,
    sync::{atomic::Ordering, mpsc::Receiver},
};

use super::{
    super::{AgentSessionError, PendingCommand},
    AgentWorker, apply_events, submission_rejection,
};
use crate::{AgentCommand, AgentEvent, RuntimeError, SubmissionId, TurnRef};

impl AgentWorker {
    pub(super) fn dispatch(
        &mut self,
        command: AgentCommand,
        submission_id: Option<SubmissionId>,
    ) -> Result<Vec<AgentEvent>, AgentSessionError> {
        self.execute(command, submission_id)
            .map_err(AgentSessionError::Runtime)
    }

    pub(super) fn execute(
        &mut self,
        command: AgentCommand,
        submission_id: Option<SubmissionId>,
    ) -> Result<Vec<AgentEvent>, RuntimeError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let AgentCommand::InterruptTurn { turn } = &command
            && self.runtime.active_turn() != Some(*turn)
        {
            // queue된 interrupt가 이미 거절되었거나 완료된 Turn보다 오래 남을 수 있습니다.
            // Session을 실패시키거나 새로운 Turn을 interrupt해서는 안 됩니다.
            if state.active_turn == Some(*turn) && !state.turn_started {
                state.active_turn = None;
                self.active_turn_id.store(0, Ordering::Release);
            }
            state.interrupted_turns.remove(turn);
            return Ok(Vec::new());
        }
        let events = if let Some(submission_id) = submission_id {
            let target = command_turn(&command);
            match self.runtime.execute_submission(command, submission_id) {
                Ok(events) => events,
                Err(error) => {
                    // 거절된 steer가 이미 commit된 Turn과 request를 남기도록
                    // 시작되지 않은 Turn 예약만 되돌립니다.
                    if submission_rejection(&error).is_some()
                        && !state.turn_started
                        && state.active_turn == target
                    {
                        if let Some(turn) = target {
                            state.interrupted_turns.remove(&turn);
                        }
                        state.active_turn = None;
                        self.active_turn_id.store(0, Ordering::Release);
                    }
                    return Err(error);
                },
            }
        } else {
            self.runtime.execute_command(command)?
        };
        apply_events(&mut state, &self.active_turn_id, &events);
        Ok(events)
    }
}

pub(in crate::agent_session) fn cancel_queued_turn_commands(
    interrupted: TurnRef,
    commands: &Receiver<PendingCommand>,
    retained: &mut VecDeque<PendingCommand>,
) -> Vec<SubmissionId> {
    let mut canceled_submissions = Vec::new();
    while let Ok(pending) = commands.try_recv() {
        if command_turn(pending.command()) != Some(interrupted) {
            retained.push_back(pending);
        } else if let Some(id) = pending.submission_id() {
            canceled_submissions.push(id);
        }
    }
    canceled_submissions
}

fn command_turn(command: &AgentCommand) -> Option<TurnRef> {
    match command {
        AgentCommand::CreateSession { .. } => None,
        AgentCommand::CompactContext { .. } => None,
        AgentCommand::StartTurn { turn, .. }
        | AgentCommand::SteerTurn { turn, .. }
        | AgentCommand::InterruptTurn { turn } => Some(*turn),
        AgentCommand::RespondToActivity { request, .. } => Some(request.activity().turn()),
    }
}
