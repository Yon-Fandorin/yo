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
    backend.promote_failure_context();
    if backend.pending_failure_context.is_some() {
        return Err(failure(
            BackendFailureKind::Session,
            "local failure context is awaiting serialized publication",
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
        AgentCommand::SteerTurn { turn, input } => backend.prepare_steer(turn, input),
        AgentCommand::InterruptTurn { turn } => backend.interrupt(turn),
        AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::Approval(decision),
        } => backend.respond_to_approval(request, decision),
        AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::SecretInput(secret),
        } => backend.respond_to_secret_input(request, secret),
        AgentCommand::RespondToActivity { request, response } => {
            backend.respond_to_question(request, response)
        },

        AgentCommand::CompactContext { guidance } => {
            backend.start_idle_compaction(guidance)?;
            Ok(BackendCommandEvidence::None)
        },
    }
}
