use std::{thread, time::Duration};

use yo_core::{
    AgentIntent, AgentSession, AgentSessionPoll, CommandAdmission, PendingCommand, SubmissionId,
    SubmissionOutcome, TranscriptObservationSequence, TranscriptReader,
};

use super::projection::{self, FinalResponseProjection};
use crate::diagnostic::AppError;

const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) fn run(
    session: &mut AgentSession,
    input: String,
    mut is_terminated: impl FnMut() -> bool,
) -> Result<String, AppError> {
    if is_terminated() {
        return Err(AppError::message("print session interrupted"));
    }
    let transcript = session.transcript_reader();
    let mut cursor = transcript.read_observations_after(None).head();
    let submission_id = SubmissionId::new()
        .map_err(|error| AppError::single("creating the print Submission", error))?;
    let intent = AgentIntent::Submit(yo_core::InputSubmission::new(
        submission_id,
        yo_core::UserInput::new(input),
    ));
    let mut pending = pending_admission(
        session
            .dispatch(intent)
            .map_err(|error| AppError::single("submitting print input", error))?,
    )?;
    let mut projection = FinalResponseProjection::default();
    let mut accepted = false;
    let mut completed = None;

    loop {
        if is_terminated() {
            return Err(AppError::message("print session interrupted"));
        }

        let mut changed = false;
        if let Some(command) = pending.take() {
            pending = pending_admission(
                session
                    .retry(command)
                    .map_err(|error| AppError::single("admitting print input", error))?,
            )?;
            changed |= pending.is_none();
        }

        let poll = session
            .poll()
            .map_err(|error| AppError::single("running the print Session", error))?;
        changed |= poll != AgentSessionPoll::Pending;

        while let Some(outcome) = session.take_submission_outcome() {
            changed = true;
            accepted |= observe_submission_outcome(outcome, submission_id)?;
        }

        changed |= drain_transcript(&transcript, &mut cursor, &mut projection, &mut completed)?;
        if accepted && let Some(message) = completed.take() {
            return Ok(projection::frame_output(message));
        }
        if poll == AgentSessionPoll::Closed {
            return Err(AppError::message(
                "print Session closed before producing a completed final response",
            ));
        }
        if !changed {
            thread::sleep(IDLE_POLL_INTERVAL);
        }
    }
}

fn observe_submission_outcome(
    outcome: SubmissionOutcome,
    expected: SubmissionId,
) -> Result<bool, AppError> {
    match outcome {
        SubmissionOutcome::Accepted { id } if id == expected => Ok(true),
        SubmissionOutcome::Rejected { id, rejection } if id == expected => Err(AppError::message(
            format!("print Submission rejected: {}", rejection.message()),
        )),
        _ => Err(AppError::message(
            "print Session returned an unrelated Submission outcome",
        )),
    }
}

fn pending_admission(admission: CommandAdmission) -> Result<Option<PendingCommand>, AppError> {
    match admission {
        CommandAdmission::Queued => Ok(None),
        CommandAdmission::Backpressured(pending) => Ok(Some(pending)),
        CommandAdmission::Rejected { rejection, .. } => Err(AppError::message(format!(
            "print Submission rejected: {}",
            rejection.message()
        ))),
    }
}

fn drain_transcript(
    transcript: &TranscriptReader,
    cursor: &mut Option<TranscriptObservationSequence>,
    projection: &mut FinalResponseProjection,
    completed: &mut Option<String>,
) -> Result<bool, AppError> {
    let mut changed = false;
    loop {
        let slice = transcript.read_observations_after(*cursor);
        let head = slice.head();
        let entries = slice.into_entries();
        if entries.is_empty() {
            break;
        }
        changed = true;
        for entry in entries {
            *cursor = Some(entry.sequence());
            if let Some(message) = projection.observe(entry.observation())? {
                *completed = Some(message);
            }
        }
        if *cursor == head {
            break;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        num::NonZeroU64,
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use yo_core::{
        ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand,
        BackendBindingEvidence, BackendCommandEvidence, BackendEvent, BackendIdentity,
        BackendOutcomeEvidence, BackendRequestEvidence, BackendScriptStep, ContextPolicyChanged,
        ContextStrategy, ContinuationStrategy, Failure, HostWorkspacePath, ModelReplayContract,
        ModelReplayDelta, ModelReplayItem, ModelReplayRole, ReplayExecutor, RequestId,
        ScriptedBackend, SessionDescriptor, SubmissionRejection, SubmissionRejectionKind, TurnId,
        TurnOutcome, TurnRef, WorkspaceHostId,
        session_repository::{
            AppendError, AppendReceipt, DurableRecord, LocalSessionRepository, RepositoryEntry,
            RepositoryError, RepositorySequence, SessionRepository, SessionWriterRepository,
            recover_stored_session_continuation,
        },
    };

    use super::*;

    // 여러 agent message와 비최종 Activity가 있어도 마지막 completed AgentMessage만
    // 반환하고 Snapshot 의미와 최종 newline framing을 보존합니다.
    #[test]
    fn runner_returns_only_the_last_completed_agent_message() {
        let fixture = SessionFixture::new("question", |turn| {
            let work = activity(turn, 1);
            let draft = activity(turn, 2);
            let final_message = activity(turn, 3);
            vec![
                BackendScriptStep::Emit(BackendEvent::ActivityStarted {
                    activity: work,
                    kind: ActivityKind::ModelWork,
                }),
                BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
                    activity: work,
                    update: ActivityUpdate::TextDelta("hidden reasoning".to_owned()),
                }),
                BackendScriptStep::Emit(BackendEvent::ActivityFinished {
                    activity: work,
                    outcome: ActivityOutcome::Completed,
                }),
                BackendScriptStep::Emit(BackendEvent::ActivityStarted {
                    activity: draft,
                    kind: ActivityKind::AgentMessage,
                }),
                BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
                    activity: draft,
                    update: ActivityUpdate::TextDelta("draft".to_owned()),
                }),
                BackendScriptStep::Emit(BackendEvent::ActivityFinished {
                    activity: draft,
                    outcome: ActivityOutcome::Completed,
                }),
                BackendScriptStep::Emit(BackendEvent::ActivityStarted {
                    activity: final_message,
                    kind: ActivityKind::AgentMessage,
                }),
                BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
                    activity: final_message,
                    update: ActivityUpdate::TextDelta("partial".to_owned()),
                }),
                BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
                    activity: final_message,
                    update: ActivityUpdate::TextSnapshot("final answer".to_owned()),
                }),
                BackendScriptStep::Emit(BackendEvent::ActivityFinished {
                    activity: final_message,
                    outcome: ActivityOutcome::Completed,
                }),
                BackendScriptStep::Emit(BackendEvent::TurnFinished {
                    turn,
                    outcome: TurnOutcome::Completed,
                }),
            ]
        });
        let (mut session, root) = fixture.start();

        assert_eq!(
            run(&mut session, "question".to_owned(), || false).unwrap(),
            "final answer\n"
        );
        session.shutdown().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    // resumed print는 dispatch 전에 기존 observation head를 cursor로 고정합니다. 복구 시
    // Journal sequence가 압축되어도 첫 Turn을 재출력하거나 두 번째 Turn을 건너뛰지 않습니다.
    #[test]
    fn resumed_runner_excludes_prior_turn_history() {
        let workspace = HostWorkspacePath::normalize_local(
            std::env::current_dir().expect("the test process has a working directory"),
        )
        .unwrap();
        let descriptor =
            SessionDescriptor::new(WorkspaceHostId::new().unwrap(), workspace).unwrap();
        let session_id = descriptor.session_id();
        let first_turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(1).unwrap()));
        let second_turn = TurnRef::new(session_id, TurnId::new(NonZeroU64::new(2).unwrap()));
        let first_backend = ScriptedBackend::new(resumable_script(
            first_turn,
            "first question",
            "first answer",
            "request-1",
            true,
        ));
        let mut repository = MemoryRepository::default();
        let mut first = AgentSession::start_cancellable_with_repository(
            first_backend,
            descriptor,
            repository.clone(),
            || false,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            run(&mut first, "first question".to_owned(), || false).unwrap(),
            "first answer\n"
        );
        first.shutdown().unwrap();
        drop(first);

        let continuation =
            recover_stored_session_continuation(&mut repository, session_id).unwrap();
        let target = continuation.target().clone();
        let mut second_steps = vec![BackendScriptStep::Resume {
            target: Box::new(target),
            evidence: resumable_binding(),
        }];
        second_steps.extend(resumable_script(
            second_turn,
            "follow up",
            "second answer",
            "request-2",
            false,
        ));
        let second_backend = ScriptedBackend::new(second_steps);
        let mut second = AgentSession::start_cancellable_with_continuation(
            second_backend,
            continuation,
            repository,
            || false,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            run(&mut second, "follow up".to_owned(), || false).unwrap(),
            "second answer\n"
        );
        second.shutdown().unwrap();
    }

    // approval이나 user-input round trip이 필요한 Turn은 답을 만들어 내거나 자동 응답하지
    // 않고 둘 다 즉시 실패하여 stdout 후보를 남기지 않습니다.
    #[test]
    fn interactive_request_fails_closed() {
        let request_id = RequestId::new(NonZeroU64::new(1).unwrap());
        for kind in [
            ActivityKind::ApprovalRequest { request_id },
            ActivityKind::UserInputRequest { request_id },
        ] {
            let fixture = SessionFixture::new("question", |turn| {
                vec![BackendScriptStep::Emit(BackendEvent::ActivityStarted {
                    activity: activity(turn, 1),
                    kind,
                })]
            });
            let (mut session, root) = fixture.start();

            let error = run(&mut session, "question".to_owned(), || false).unwrap_err();
            assert!(error.to_string().contains("interactive response"));
            session.shutdown().unwrap();
            fs::remove_dir_all(root).unwrap();
        }
    }

    // 정확한 Submission 거절은 해당 이유를 보존하고, 다른 Submission의 outcome은 현재
    // 호출의 수락으로 오인하지 않아 두 경우 모두 output eligibility를 열지 않습니다.
    #[test]
    fn submission_outcomes_are_identity_bound_and_rejection_is_distinct() {
        let expected = SubmissionId::new().unwrap();
        let other = SubmissionId::new().unwrap();

        assert!(
            observe_submission_outcome(SubmissionOutcome::Accepted { id: expected }, expected)
                .unwrap()
        );
        let rejection = observe_submission_outcome(
            SubmissionOutcome::Rejected {
                id: expected,
                rejection: SubmissionRejection::new(
                    SubmissionRejectionKind::OverBudget,
                    "too large",
                ),
            },
            expected,
        )
        .unwrap_err();
        assert!(rejection.to_string().contains("rejected: too large"));
        let unrelated =
            observe_submission_outcome(SubmissionOutcome::Accepted { id: other }, expected)
                .unwrap_err();
        assert!(unrelated.to_string().contains("unrelated Submission"));
    }

    // worker queue에 들어가기 전 동기 admission 거절도 비동기 outcome과 같은 이유를
    // 보고하고, print loop가 이를 성공적으로 queue된 command로 기다리지 않게 합니다.
    #[test]
    fn synchronous_submission_rejection_stops_print_admission() {
        let error = pending_admission(CommandAdmission::Rejected {
            id: SubmissionId::new().unwrap(),
            rejection: SubmissionRejection::new(
                SubmissionRejectionKind::StaleReference,
                "the target Turn ended",
            ),
        })
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("rejected: the target Turn ended")
        );
    }

    // Turn interruption과 failure는 서로 구분되는 진단으로 끝나며, 그 전에 completed
    // message가 있었더라도 성공 output으로 반환하지 않습니다.
    #[test]
    fn interrupted_and_failed_turns_remain_distinct_failures() {
        for (outcome, expected) in [
            (TurnOutcome::Interrupted, "was interrupted"),
            (
                TurnOutcome::Failed(Failure::new("backend stopped")),
                "backend stopped",
            ),
        ] {
            let fixture = SessionFixture::new("question", |turn| {
                let message = activity(turn, 1);
                vec![
                    BackendScriptStep::Emit(BackendEvent::ActivityStarted {
                        activity: message,
                        kind: ActivityKind::AgentMessage,
                    }),
                    BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
                        activity: message,
                        update: ActivityUpdate::TextSnapshot("ineligible".to_owned()),
                    }),
                    BackendScriptStep::Emit(BackendEvent::ActivityFinished {
                        activity: message,
                        outcome: ActivityOutcome::Completed,
                    }),
                    BackendScriptStep::Emit(BackendEvent::TurnFinished { turn, outcome }),
                ]
            });
            let (mut session, root) = fixture.start();

            let error = run(&mut session, "question".to_owned(), || false).unwrap_err();
            assert!(error.to_string().contains(expected));
            session.shutdown().unwrap();
            fs::remove_dir_all(root).unwrap();
        }
    }

    // completed Turn에 completed AgentMessage가 없으면 빈 성공으로 오인하지 않고 명시적
    // 실패가 되어 기본 stdout eligibility를 닫습니다.
    #[test]
    fn completed_turn_without_final_message_fails() {
        let fixture = SessionFixture::new("question", |turn| {
            vec![BackendScriptStep::Emit(BackendEvent::TurnFinished {
                turn,
                outcome: TurnOutcome::Completed,
            })]
        });
        let (mut session, root) = fixture.start();

        let error = run(&mut session, "question".to_owned(), || false).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("without a completed AgentMessage")
        );
        session.shutdown().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    struct SessionFixture {
        descriptor: SessionDescriptor,
        root: PathBuf,
        steps: Vec<BackendScriptStep>,
    }

    #[derive(Clone, Debug, Default)]
    struct MemoryRepository {
        entries: Arc<Mutex<Vec<RepositoryEntry>>>,
    }

    impl SessionRepository for MemoryRepository {
        fn append(
            &mut self,
            _session_id: yo_core::SessionId,
            record: DurableRecord,
        ) -> Result<AppendReceipt, AppendError> {
            let mut entries = self.entries.lock().unwrap();
            let sequence = RepositorySequence::new(u64::try_from(entries.len()).unwrap() + 1);
            entries.push(RepositoryEntry::new(sequence, record));
            Ok(AppendReceipt::new(sequence))
        }

        fn read_after(
            &self,
            _session_id: yo_core::SessionId,
            sequence: Option<RepositorySequence>,
            limit: usize,
        ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
            let after = sequence.map_or(0, RepositorySequence::get);
            Ok(self
                .entries
                .lock()
                .unwrap()
                .iter()
                .filter(|entry| entry.sequence().get() > after)
                .take(limit)
                .cloned()
                .collect())
        }
    }

    impl SessionWriterRepository for MemoryRepository {
        fn acquire_session_writer(
            &mut self,
            _session_id: yo_core::SessionId,
        ) -> Result<(), RepositoryError> {
            Ok(())
        }
    }

    impl SessionFixture {
        fn new(input: &str, later: impl FnOnce(TurnRef) -> Vec<BackendScriptStep>) -> Self {
            let workspace = HostWorkspacePath::normalize_local(
                std::env::current_dir().expect("the test process has a working directory"),
            )
            .unwrap();
            let descriptor =
                SessionDescriptor::new(WorkspaceHostId::new().unwrap(), workspace).unwrap();
            let turn = TurnRef::new(
                descriptor.session_id(),
                TurnId::new(NonZeroU64::new(1).unwrap()),
            );
            let root = std::env::temp_dir()
                .join(format!("yo-print-mode-test-{}", descriptor.session_id()));
            let mut steps = vec![
                BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
                    session_id: descriptor.session_id(),
                }),
                BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
                    turn,
                    input: yo_core::UserInput::new(input),
                }),
            ];
            steps.extend(later(turn));
            steps.push(BackendScriptStep::Shutdown(Ok(())));
            Self {
                descriptor,
                root,
                steps,
            }
        }

        fn start(self) -> (AgentSession, PathBuf) {
            let repository = LocalSessionRepository::open(&self.root, 1024 * 1024).unwrap();
            let session = AgentSession::start_cancellable_with_repository(
                ScriptedBackend::new(self.steps),
                self.descriptor,
                repository,
                || false,
            )
            .unwrap()
            .unwrap();
            (session, self.root)
        }
    }

    fn activity(turn: TurnRef, id: u64) -> ActivityRef {
        ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(id).unwrap()))
    }

    fn resumable_binding() -> BackendBindingEvidence {
        BackendBindingEvidence::new(
            "managed",
            "test/v1",
            BackendIdentity::new("managed.session/v1", "session"),
            BackendIdentity::new(
                "managed.model-service-binding/v1",
                "managed:qwencloud:default:qwen3.8-max",
            ),
            BackendIdentity::new("managed.session-locator/v1", "session"),
            ContinuationStrategy::ExactReplay {
                executor: ReplayExecutor::LocalClient,
                replay_profile: yo_core::ReplayProfile::SemanticOnly,
            },
        )
    }

    fn resumable_script(
        turn: TurnRef,
        input: &str,
        answer: &str,
        request_id: &str,
        create: bool,
    ) -> Vec<BackendScriptStep> {
        let mut steps = Vec::new();
        if create {
            steps.push(BackendScriptStep::AcceptCommandWithEvidence {
                command: AgentCommand::CreateSession {
                    session_id: turn.session_id(),
                },
                evidence: BackendCommandEvidence::BindingOpened(resumable_binding()),
            });
            steps.push(BackendScriptStep::Emit(
                BackendEvent::ContextPolicyChanged {
                    policy: ContextPolicyChanged::try_new(
                        1,
                        true,
                        ContextStrategy::PortableSummaryV1Alpha1,
                        85,
                        90,
                        Some(10),
                        Some(65_536),
                    )
                    .unwrap(),
                },
            ));
        }
        steps.push(BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::StartTurn {
                turn,
                input: yo_core::UserInput::new(input),
            },
            evidence: BackendCommandEvidence::RequestAccepted(BackendRequestEvidence::new(
                "managed/request/v1",
                BackendIdentity::new("managed.request/v1", request_id),
                BackendIdentity::new("managed.accepted-request/v1", request_id),
            )),
        });
        let message = activity(turn, 1);
        steps.extend([
            BackendScriptStep::Emit(BackendEvent::ActivityStarted {
                activity: message,
                kind: ActivityKind::AgentMessage,
            }),
            BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
                activity: message,
                update: ActivityUpdate::TextSnapshot(answer.to_owned()),
            }),
            BackendScriptStep::Emit(BackendEvent::ActivityFinished {
                activity: message,
                outcome: ActivityOutcome::Completed,
            }),
            BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
                turn,
                evidence: BackendOutcomeEvidence::with_identity(BackendIdentity::new(
                    "managed.outcome/v1",
                    request_id,
                ))
                .with_replay(ModelReplayDelta::new(
                    create.then(|| ModelReplayContract::new("system", Vec::new())),
                    vec![
                        ModelReplayItem::Message {
                            role: ModelReplayRole::User,
                            content: input.to_owned(),
                            refusal: None,
                        },
                        ModelReplayItem::Message {
                            role: ModelReplayRole::Assistant,
                            content: answer.to_owned(),
                            refusal: None,
                        },
                    ],
                )),
            }),
            BackendScriptStep::Shutdown(Ok(())),
        ]);
        steps
    }
}
