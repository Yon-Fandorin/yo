//! 명령과 제출 디스패치.

use yo_core::{
    ActivityResponse, AgentCommand, BackendCommandEvidence, BackendFailure, BackendFailureKind,
};

use super::super::{NativeModelBackend, failure};

pub(super) fn execute_command(
    backend: &mut NativeModelBackend,
    command: AgentCommand,
) -> Result<BackendCommandEvidence, BackendFailure> {
    if backend.closed {
        return Err(failure(
            BackendFailureKind::Session,
            "native backend is closed",
        ));
    }
    match command {
        AgentCommand::CreateSession { session_id } => {
            if backend.session.is_some() {
                return Err(failure(
                    BackendFailureKind::Session,
                    "native backend already has a Session",
                ));
            }
            backend.session = Some(session_id);
            backend.context_policy_active = true;
            backend
                .events
                .push_back(yo_core::BackendEvent::ContextPolicyChanged {
                    policy: backend.config.context_policy.clone(),
                });
            Ok(BackendCommandEvidence::BindingOpened(
                backend.binding_evidence(session_id),
            ))
        },
        AgentCommand::StartTurn { turn, input } => {
            backend.start_turn(turn, input.model_replay_item())
        },
        AgentCommand::SteerTurn { .. } => Err(failure(
            BackendFailureKind::Unsupported,
            "native model loop does not support steering",
        )),
        AgentCommand::InterruptTurn { turn } => backend.interrupt(turn),
        AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::Approval(decision),
        } => backend.respond_to_approval(request, decision),
        AgentCommand::RespondToActivity { .. } => Err(failure(
            BackendFailureKind::Unsupported,
            "native model loop only accepts approval responses",
        )),
        AgentCommand::CompactContext { guidance } => {
            backend.start_idle_compaction(guidance)?;
            Ok(BackendCommandEvidence::None)
        },
    }
}
