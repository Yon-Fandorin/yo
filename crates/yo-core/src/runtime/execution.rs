use std::mem;

use super::{
    AgentBackend, AgentRuntime, RuntimeError,
    replacement::{command_kind, submission_turn},
};
use crate::{
    ActivityResponse, AgentCommand, AgentEvent, BackendCommandEvidence, BackendEvent,
    BackendFailureKind, BackendPoll, ContinuationStrategy, ImageInputCapability, InputImageHistory,
    SubmissionId, SubmissionRejection, SubmissionRejectionKind, journal::ContextActiveSource,
};

impl<B: AgentBackend> AgentRuntime<B> {
    /// Validates a command, lets the backend accept it, then commits its semantic transition.
    pub fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<Vec<AgentEvent>, RuntimeError> {
        if matches!(
            command,
            AgentCommand::StartTurn { .. } | AgentCommand::SteerTurn { .. }
        ) {
            return Err(RuntimeError::SubmissionIdentityRequired);
        }
        self.execute(command, None)
    }

    pub fn execute_submission(
        &mut self,
        command: AgentCommand,
        submission_id: SubmissionId,
    ) -> Result<Vec<AgentEvent>, RuntimeError> {
        if !matches!(
            command,
            AgentCommand::StartTurn { .. } | AgentCommand::SteerTurn { .. }
        ) {
            return Err(RuntimeError::SubmissionIdentityUnexpected);
        }
        if self.submission_ids.contains(&submission_id) {
            return Err(RuntimeError::DuplicateSubmissionIdentity(submission_id));
        }
        self.input_admission_sealed = true;
        self.execute(command, Some(submission_id))
    }

    fn execute(
        &mut self,
        mut command: AgentCommand,
        submission_id: Option<SubmissionId>,
    ) -> Result<Vec<AgentEvent>, RuntimeError> {
        if self.secret_input_terminal && !matches!(command, AgentCommand::InterruptTurn { .. }) {
            return Err(RuntimeError::backend(crate::BackendFailure::new(
                BackendFailureKind::Session,
                "this Session ended at a protected input submission and cannot accept more commands",
            )));
        }
        if matches!(command, AgentCommand::StartTurn { .. })
            && !self.context_policy_initialized
            && matches!(
                self.continuation_strategy,
                Some(ContinuationStrategy::ExactReplay {
                    executor: crate::ReplayExecutor::LocalClient,
                    ..
                })
            )
        {
            let poll = self
                .backend
                .poll_event()
                .map_err(|failure| RuntimeError::backend(self.redact_backend_failure(failure)))?;
            match poll {
                BackendPoll::Event(event @ BackendEvent::ContextPolicyChanged { .. }) => {
                    self.apply_backend_event(event)?;
                },
                BackendPoll::Event(_) | BackendPoll::Pending | BackendPoll::Closed => {
                    return Err(RuntimeError::backend(crate::BackendFailure::new(
                        BackendFailureKind::Protocol,
                        "local-client exact replay did not publish its context policy before the first model request",
                    )));
                },
            }
        }
        let supports_steer = self.backend.capabilities().supports_steer();
        self.engine
            .validate_command(&command, supports_steer)
            .map_err(RuntimeError::CommandRejected)?;
        if let AgentCommand::RespondToActivity { response, .. } = &command
            && (response.has_resolved_skill() || response.has_images())
        {
            return Err(RuntimeError::InputRejected(SubmissionRejection::new(
                SubmissionRejectionKind::InvalidReference,
                "Activity responses cannot contain images or resolved skill instructions",
            )));
        }
        if matches!(
            command,
            AgentCommand::RespondToActivity {
                response: ActivityResponse::SecretInputSubmitted,
                ..
            }
        ) {
            return Err(RuntimeError::InputRejected(SubmissionRejection::new(
                SubmissionRejectionKind::InvalidReference,
                "a secret-input receipt cannot be dispatched as a live response",
            )));
        }
        let historical_image_guard = matches!(
            self.continuation_strategy,
            Some(ContinuationStrategy::BackendManagedState)
        ) && self.input_image_history != InputImageHistory::TextOnly;
        if let AgentCommand::StartTurn { input, .. } | AgentCommand::SteerTurn { input, .. } =
            &mut command
            && (!input.references().is_empty()
                || !input.images().is_empty()
                || historical_image_guard)
        {
            if !input.images().is_empty() || historical_image_guard {
                match self.backend.capabilities().image_input() {
                    ImageInputCapability::Unknown => {
                        return Err(RuntimeError::InputRejected(SubmissionRejection::new(
                            SubmissionRejectionKind::ImageCapabilityUnknown,
                            "Image support has not been established for the selected backend and model",
                        )));
                    },
                    ImageInputCapability::Unsupported => {
                        return Err(RuntimeError::InputRejected(SubmissionRejection::new(
                            SubmissionRejectionKind::ImageUnsupported,
                            "The selected backend and model do not admit image input",
                        )));
                    },
                    ImageInputCapability::Supported {
                        maximum_occurrences,
                        maximum_image_bytes,
                        maximum_input_bytes,
                    } => {
                        let total = input.images().iter().try_fold(0_u64, |total, image| {
                            total.checked_add(image.snapshot().png().len() as u64)
                        });
                        if input.images().len() as u64 > u64::from(maximum_occurrences)
                            || input.images().iter().any(|image| {
                                image.snapshot().png().len() as u64 > maximum_image_bytes
                            })
                            || total.is_none_or(|total| total > maximum_input_bytes)
                        {
                            return Err(RuntimeError::InputRejected(SubmissionRejection::new(
                                SubmissionRejectionKind::OverBudget,
                                "The complete image input exceeds the selected model's admitted limits",
                            )));
                        }
                    },
                }
            }
            if !input.references().is_empty() || !input.images().is_empty() {
                let host = self.input_admission.get().ok_or_else(|| {
                    RuntimeError::InputRejected(SubmissionRejection::new(
                        SubmissionRejectionKind::EnvironmentUnavailable,
                        "the Session has no execution host for structured input admission",
                    ))
                })?;
                if input.resolved_skill().is_some() {
                    return Err(RuntimeError::InputRejected(SubmissionRejection::new(
                        SubmissionRejectionKind::InvalidReference,
                        "live input cannot supply pre-resolved skill instructions",
                    )));
                }
                host.validate_images(input)
                    .map_err(RuntimeError::InputRejected)?;
                let resolved = host.prepare(input).map_err(RuntimeError::InputRejected)?;
                if let Some(skill) = resolved {
                    *input = input.clone().with_resolved_skill(skill).map_err(|error| {
                        RuntimeError::InputRejected(SubmissionRejection::new(
                            SubmissionRejectionKind::InvalidReference,
                            error.to_string(),
                        ))
                    })?;
                } else if input
                    .references()
                    .iter()
                    .any(|reference| reference.skill_reference().is_some())
                {
                    return Err(RuntimeError::InputRejected(SubmissionRejection::new(
                        SubmissionRejectionKind::RequiredAssetUnavailable,
                        "the execution host did not resolve the selected skill",
                    )));
                }
            }
        }
        let turn_submission = matches!(
            command,
            AgentCommand::StartTurn { .. } | AgentCommand::SteerTurn { .. }
        );
        let live_secret_response = matches!(
            command,
            AgentCommand::RespondToActivity {
                response: ActivityResponse::SecretInput(_),
                ..
            }
        );
        let backend_command = if live_secret_response {
            let AgentCommand::RespondToActivity { request, response } = &mut command else {
                unreachable!("the guarded live secret command is an Activity response");
            };
            let response = mem::replace(response, ActivityResponse::SecretInputSubmitted);
            AgentCommand::RespondToActivity {
                request: *request,
                response,
            }
        } else {
            command.clone()
        };
        if live_secret_response {
            // The backend call may write the value before reporting any failure. From this
            // boundary onward, backend-owned diagnostics are unsafe for public or durable use.
            self.secret_diagnostics_redacted = true;
        }
        let evidence = match self.backend.execute_command(backend_command) {
            Err(failure) => {
                let kind = failure.kind();
                if live_secret_response && kind == BackendFailureKind::InputOverBudget {
                    return Err(RuntimeError::InputRejected(SubmissionRejection::new(
                        SubmissionRejectionKind::OverBudget,
                        "secret interview input exceeds the 256 KiB live batch limit",
                    )));
                }
                if live_secret_response {
                    return Err(RuntimeError::backend(crate::BackendFailure::new(
                        kind,
                        "secret input delivery failed with an unknown outcome",
                    )));
                }
                let failure = self.redact_backend_failure(failure);
                if turn_submission && kind == BackendFailureKind::InputOverBudget {
                    return Err(RuntimeError::InputRejected(SubmissionRejection::new(
                        SubmissionRejectionKind::OverBudget,
                        failure.message(),
                    )));
                }
                return Err(RuntimeError::backend(failure));
            },
            Ok(evidence) => evidence,
        };
        if let Err(error) = self.validate_command_evidence(&command, submission_id, &evidence) {
            if submission_id.is_some() {
                let turn = submission_turn(&command);
                self.accepted_requests.remove(&turn);
                self.accepted_submissions.remove(&turn);
            }
            return Err(error);
        }
        let committed = command.clone();
        let starts_idle_context_compaction =
            matches!(&committed, AgentCommand::CompactContext { .. });
        let command_sequence = self.journal.next_sequence();
        let active_input = match &committed {
            AgentCommand::StartTurn { turn, input } => Some((*turn, input.model_replay_item())),
            _ => None,
        };
        let commits_image_input = matches!(
            &committed,
            AgentCommand::StartTurn { input, .. } | AgentCommand::SteerTurn { input, .. }
                if !input.images().is_empty()
        );
        let events = match self.engine.commit_command(command, supports_steer) {
            Ok(events) => events,
            Err(rejection) => {
                if matches!(evidence, BackendCommandEvidence::ProtectedInputPrepared) {
                    let _ = self.backend.abort_prepared_command();
                    self.secret_input_terminal = true;
                }
                return Err(RuntimeError::StateDiverged(rejection));
            },
        };
        match (submission_id, evidence) {
            (Some(submission_id), BackendCommandEvidence::RequestAccepted(evidence)) => {
                let inserted = self.submission_ids.insert(submission_id);
                debug_assert!(inserted, "a duplicate submission cannot pass validation");
                let epoch = self
                    .binding_epoch
                    .expect("request evidence validation requires an open binding");
                let turn = submission_turn(&committed);
                let accepted = self.journal.append_accepted_submission(
                    committed,
                    submission_id,
                    &events,
                    epoch,
                    self.context_epoch,
                    evidence,
                );
                self.accepted_requests.insert(turn, accepted);
                self.accepted_submissions.insert(turn, submission_id);
                self.binding_has_accepted_request = true;
                self.binding_has_unanchored_request = true;
            },
            (Some(submission_id), BackendCommandEvidence::None) => {
                let inserted = self.submission_ids.insert(submission_id);
                debug_assert!(inserted, "a duplicate submission cannot pass validation");
                let turn = submission_turn(&committed);
                self.accepted_requests.remove(&turn);
                self.accepted_submissions.insert(turn, submission_id);
                self.journal
                    .append_committed_submission(committed, submission_id, &events);
            },
            (None, BackendCommandEvidence::BindingOpened(evidence)) => {
                let epoch = 1;
                self.input_image_history = InputImageHistory::TextOnly;
                self.binding = Some(evidence.clone());
                self.continuation_strategy = Some(evidence.continuation_strategy());
                self.journal
                    .append_initial_binding(committed, &events, epoch, evidence);
                self.binding_epoch = Some(epoch);
                self.binding_has_accepted_request = false;
                self.binding_has_unanchored_request = false;
            },
            (None, BackendCommandEvidence::None) => {
                self.journal.append_committed_command(committed, &events);
            },
            (None, BackendCommandEvidence::ProtectedInputPrepared) => {
                self.secret_input_terminal = true;
                if !self
                    .journal
                    .append_committed_command_transactionally(committed, &events)
                {
                    let _ = self.backend.abort_prepared_command();
                    return Err(RuntimeError::backend(crate::BackendFailure::new(
                        BackendFailureKind::Session,
                        "protected input receipt could not be committed durably",
                    )));
                }
                if self.backend.commit_prepared_command().is_err() {
                    let _ = self.backend.abort_prepared_command();
                    return Err(RuntimeError::backend(crate::BackendFailure::new(
                        BackendFailureKind::Session,
                        "protected input was committed but its terminal request could not be armed",
                    )));
                }
            },
            _ => unreachable!("command evidence was validated before semantic commit"),
        }
        if starts_idle_context_compaction {
            self.idle_context_compaction_pending = true;
            self.idle_context_checkpoint_committed = false;
        }
        if commits_image_input {
            self.input_image_history = InputImageHistory::ContainsImages;
        }
        if let Some((turn, item)) = active_input {
            self.active_context_source = Some(ContextActiveSource::new(
                turn,
                command_sequence,
                command_sequence,
                vec![item],
            ));
        }
        Ok(events)
    }

    fn validate_command_evidence(
        &self,
        command: &AgentCommand,
        submission_id: Option<SubmissionId>,
        evidence: &BackendCommandEvidence,
    ) -> Result<(), RuntimeError> {
        let valid = match evidence {
            BackendCommandEvidence::None => true,
            BackendCommandEvidence::BindingOpened(evidence) => {
                matches!(command, AgentCommand::CreateSession { .. })
                    && submission_id.is_none()
                    && self.binding_epoch.is_none()
                    && evidence.is_valid()
            },
            BackendCommandEvidence::RequestAccepted(evidence) => {
                matches!(
                    command,
                    AgentCommand::StartTurn { .. } | AgentCommand::SteerTurn { .. }
                ) && submission_id.is_some()
                    && self.binding_epoch.is_some()
                    && evidence.is_valid()
            },
            BackendCommandEvidence::ProtectedInputPrepared => {
                matches!(
                    command,
                    AgentCommand::RespondToActivity {
                        response: ActivityResponse::SecretInputSubmitted,
                        ..
                    }
                ) && submission_id.is_none()
            },
        };
        if valid {
            return Ok(());
        }
        Err(RuntimeError::backend(crate::BackendFailure::new(
            BackendFailureKind::Protocol,
            format!(
                "backend returned correlation evidence incompatible with {}",
                command_kind(command)
            ),
        )))
    }
}
