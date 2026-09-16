use std::mem;

use super::{AgentBackend, AgentRuntime, BackendReplacementError, RuntimeError};
use crate::{
    AgentCommand, BackendBindingEvidence, BackendFailureKind, BackendResumeSource,
    BackendResumeTarget, ContinuationStrategy, ModelReplay, TurnRef,
};

impl AgentRuntime<Box<dyn AgentBackend + Send>> {
    /// Replaces one idle exact-replay binding without releasing the current backend first.
    ///
    /// The candidate is resumed and its transition is durably committed before it becomes active.
    /// Any pre-commit failure cleans up only the candidate, leaving the current backend usable.
    pub(crate) fn replace_backend(
        &mut self,
        mut candidate: Box<dyn AgentBackend + Send>,
    ) -> Result<Option<crate::BackendFailure>, BackendReplacementError> {
        if self.engine.active_turn().is_some() {
            return Err(reject_replacement_candidate(
                &mut candidate,
                RuntimeError::backend(crate::BackendFailure::new(
                    BackendFailureKind::Session,
                    "binding replacement requires an idle Session",
                )),
            ));
        }
        let (Some(session_id), Some(epoch), Some(binding)) = (
            self.engine.session_id(),
            self.binding_epoch,
            self.binding.clone(),
        ) else {
            return Err(reject_replacement_candidate(
                &mut candidate,
                RuntimeError::backend(crate::BackendFailure::new(
                    BackendFailureKind::Session,
                    "binding replacement requires one open durable binding",
                )),
            ));
        };
        let Some(next_epoch) = epoch.checked_add(1) else {
            return Err(reject_replacement_candidate(
                &mut candidate,
                RuntimeError::backend(crate::BackendFailure::new(
                    BackendFailureKind::Session,
                    "backend binding epoch is exhausted",
                )),
            ));
        };

        let evidence = match binding.continuation_strategy() {
            ContinuationStrategy::ExactReplay { .. } => {
                let Some(resume_source) = self.resume_source else {
                    return Err(reject_replacement_candidate(
                        &mut candidate,
                        RuntimeError::backend(crate::BackendFailure::new(
                            BackendFailureKind::Session,
                            "exact-replay replacement requires one durable continuation source",
                        )),
                    ));
                };
                let target = match resume_source {
                    BackendResumeSource::ContinuationAnchor(sequence) => {
                        BackendResumeTarget::new(session_id, epoch, binding.clone(), sequence)
                    },
                    BackendResumeSource::ContextCheckpoint(sequence) => {
                        BackendResumeTarget::from_checkpoint(
                            session_id,
                            epoch,
                            binding.clone(),
                            sequence,
                        )
                    },
                    BackendResumeSource::InitialFork(sequence) => {
                        BackendResumeTarget::from_initial_fork(
                            session_id,
                            epoch,
                            binding.clone(),
                            sequence,
                        )
                    },
                }
                .with_model_replay(self.model_replay.clone())
                .with_input_image_history(self.input_image_history)
                .with_context_state(
                    self.context_policy.clone(),
                    self.context_epoch,
                    self.context_replay_groups.clone(),
                )
                .with_replay_contract_rebind_required(self.replay_contract_rebind_required)
                .with_binding_has_accepted_request(self.binding_has_accepted_request);
                let evidence = candidate
                    .resume_session_replacing_binding(&target)
                    .map_err(RuntimeError::backend)
                    .map_err(|primary| reject_replacement_candidate(&mut candidate, primary))?;
                if !valid_replacement_binding(&binding, &evidence, &self.model_replay) {
                    return Err(reject_replacement_candidate(
                        &mut candidate,
                        RuntimeError::backend(crate::BackendFailure::new(
                            BackendFailureKind::Session,
                            "exact-replay replacement returned an incompatible target binding",
                        )),
                    ));
                }
                if !self.journal.commit_exact_replay_replacement(
                    epoch,
                    next_epoch,
                    resume_source,
                    evidence.clone(),
                ) {
                    return Err(reject_replacement_candidate(
                        &mut candidate,
                        RuntimeError::backend(crate::BackendFailure::new(
                            BackendFailureKind::Session,
                            "exact-replay replacement could not publish its binding transition",
                        )),
                    ));
                }
                evidence
            },
            ContinuationStrategy::BackendManagedState => {
                if !self.backend.capabilities().supports_native_model_rebind()
                    || !candidate.capabilities().supports_native_model_rebind()
                {
                    return Err(reject_replacement_candidate(
                        &mut candidate,
                        RuntimeError::backend(crate::BackendFailure::new(
                            BackendFailureKind::Unsupported,
                            "source and candidate backends must advertise native model rebinding",
                        )),
                    ));
                }
                let source_anchor = if self.binding_has_accepted_request
                    || matches!(
                        self.resume_source,
                        Some(BackendResumeSource::InitialFork(_))
                    ) {
                    if self.binding_has_unanchored_request {
                        return Err(reject_replacement_candidate(
                            &mut candidate,
                            RuntimeError::backend(crate::BackendFailure::new(
                                BackendFailureKind::Session,
                                "native model rebinding requires the newest durable continuation Anchor after an accepted request",
                            )),
                        ));
                    }
                    match self.resume_source {
                        Some(BackendResumeSource::ContinuationAnchor(sequence)) => Some(sequence),
                        Some(
                            BackendResumeSource::ContextCheckpoint(_)
                            | BackendResumeSource::InitialFork(_),
                        )
                        | None => {
                            return Err(reject_replacement_candidate(
                                &mut candidate,
                                RuntimeError::backend(crate::BackendFailure::new(
                                    BackendFailureKind::Session,
                                    "native model rebinding requires the newest durable continuation Anchor after an accepted request",
                                )),
                            ));
                        },
                    }
                } else {
                    None
                };
                let target = BackendResumeTarget::for_model_rebind(
                    session_id,
                    epoch,
                    binding.clone(),
                    source_anchor,
                );
                let target = target.with_input_image_history(self.input_image_history);
                let evidence = candidate
                    .resume_session_rebinding_model(&target)
                    .map_err(RuntimeError::backend)
                    .map_err(|primary| reject_replacement_candidate(&mut candidate, primary))?;
                if !valid_native_model_rebind(&binding, &evidence) {
                    return Err(reject_replacement_candidate(
                        &mut candidate,
                        RuntimeError::backend(crate::BackendFailure::new(
                            BackendFailureKind::Session,
                            "native model rebind returned an incompatible target binding",
                        )),
                    ));
                }
                if !self.journal.commit_native_model_rebind(
                    epoch,
                    next_epoch,
                    source_anchor,
                    evidence.clone(),
                ) {
                    return Err(reject_replacement_candidate(
                        &mut candidate,
                        RuntimeError::backend(crate::BackendFailure::new(
                            BackendFailureKind::Session,
                            "native model rebind could not publish its binding transition",
                        )),
                    ));
                }
                self.resume_source = source_anchor.map(BackendResumeSource::ContinuationAnchor);
                self.binding_has_accepted_request = false;
                self.binding_has_unanchored_request = false;
                evidence
            },
        };

        self.binding_epoch = Some(next_epoch);
        self.binding = Some(evidence.clone());
        self.continuation_strategy = Some(evidence.continuation_strategy());
        self.binding_has_accepted_request = false;
        self.binding_has_unanchored_request = false;
        if evidence.continuation_strategy() == ContinuationStrategy::BackendManagedState {
            self.model_replay = ModelReplay::default();
            self.context_replay_groups.clear();
            self.replay_contract_rebind_required = false;
        } else {
            self.replay_contract_rebind_required = true;
        }
        let mut previous = mem::replace(&mut self.backend, candidate);
        Ok(previous.shutdown().err())
    }
}

pub(super) fn valid_replacement_binding(
    previous: &BackendBindingEvidence,
    replacement: &BackendBindingEvidence,
    replay: &ModelReplay,
) -> bool {
    if !replacement.is_valid()
        || replacement.backend_kind() != previous.backend_kind()
        || replacement.session_locator() != previous.session_locator()
    {
        return false;
    }
    let has_provider_private = replay.items().iter().any(|item| {
        matches!(
            item,
            crate::ModelReplayItem::ProviderPrivateAssistant { .. }
        )
    });
    if !has_provider_private {
        return replacement.binding_identity() != previous.binding_identity();
    }
    let replay_profile = |strategy| match strategy {
        ContinuationStrategy::ExactReplay { replay_profile, .. } => Some(replay_profile),
        ContinuationStrategy::BackendManagedState => None,
    };
    replacement.binding_identity() == previous.binding_identity()
        && replay_profile(replacement.continuation_strategy())
            == replay_profile(previous.continuation_strategy())
}

fn valid_native_model_rebind(
    previous: &BackendBindingEvidence,
    replacement: &BackendBindingEvidence,
) -> bool {
    replacement.is_valid()
        && previous.continuation_strategy() == ContinuationStrategy::BackendManagedState
        && replacement.continuation_strategy() == ContinuationStrategy::BackendManagedState
        && replacement.backend_kind() == previous.backend_kind()
        && replacement.binding_identity() != previous.binding_identity()
        && replacement.session_locator() != previous.session_locator()
        && replacement.model_identity() != previous.model_identity()
}

fn reject_replacement_candidate(
    candidate: &mut Box<dyn AgentBackend + Send>,
    primary: RuntimeError,
) -> BackendReplacementError {
    BackendReplacementError {
        primary,
        cleanup_failure: candidate.shutdown().err(),
    }
}

pub(super) fn submission_turn(command: &AgentCommand) -> TurnRef {
    match command {
        AgentCommand::StartTurn { turn, .. } | AgentCommand::SteerTurn { turn, .. } => *turn,
        _ => unreachable!("only a submission command has an accepted request"),
    }
}

pub(super) fn command_kind(command: &AgentCommand) -> &'static str {
    match command {
        AgentCommand::CreateSession { .. } => "CreateSession",
        AgentCommand::StartTurn { .. } => "StartTurn",
        AgentCommand::SteerTurn { .. } => "SteerTurn",
        AgentCommand::InterruptTurn { .. } => "InterruptTurn",
        AgentCommand::RespondToActivity { .. } => "RespondToActivity",
        AgentCommand::CompactContext { .. } => "CompactContext",
    }
}
