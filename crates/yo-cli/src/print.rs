use std::{
    collections::HashMap,
    io::{IsTerminal, Read},
    thread,
    time::Duration,
};

use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand, AgentEvent,
    AgentIntent, AgentSession, AgentSessionPoll, CommandAdmission, JournalSequence, PendingCommand,
    SubmissionId, SubmissionOutcome, TranscriptReader, TranscriptRecord, TurnOutcome, TurnRef,
};

use crate::diagnostic::AppError;

const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) fn read_input(prompt: Option<String>) -> Result<String, AppError> {
    let stdin = std::io::stdin();
    let is_terminal = stdin.is_terminal();
    let mut stdin = stdin.lock();
    read_input_from(prompt, &mut stdin, is_terminal)
}

fn read_input_from(
    prompt: Option<String>,
    stdin: &mut impl Read,
    stdin_is_terminal: bool,
) -> Result<String, AppError> {
    let stdin_text = if stdin_is_terminal {
        String::new()
    } else {
        let mut bytes = Vec::new();
        stdin
            .read_to_end(&mut bytes)
            .map_err(|error| AppError::single("reading print input from stdin", error))?;
        String::from_utf8(bytes).map_err(|error| {
            AppError::single("reading UTF-8 print input from stdin", error.utf8_error())
        })?
    };
    compose_input(stdin_text, prompt)
}

fn compose_input(stdin: String, prompt: Option<String>) -> Result<String, AppError> {
    let prompt = prompt.unwrap_or_default();
    let input = match (stdin.is_empty(), prompt.is_empty()) {
        (true, true) => {
            return Err(AppError::message(
                "print mode requires a positional prompt or non-empty piped stdin",
            ));
        },
        (false, true) => stdin,
        (true, false) => prompt,
        (false, false) if stdin.ends_with('\n') => stdin + &prompt,
        (false, false) => stdin + "\n" + &prompt,
    };
    Ok(input)
}

pub(crate) fn run(
    session: &mut AgentSession,
    input: String,
    mut is_terminated: impl FnMut() -> bool,
) -> Result<String, AppError> {
    if is_terminated() {
        return Err(AppError::message("print session interrupted"));
    }
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
    );
    let transcript = session.transcript_reader();
    let mut cursor = None;
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
            );
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
            return Ok(frame_output(message));
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

fn pending_admission(admission: CommandAdmission) -> Option<PendingCommand> {
    match admission {
        CommandAdmission::Queued => None,
        CommandAdmission::Backpressured(pending) => Some(pending),
    }
}

fn drain_transcript(
    transcript: &TranscriptReader,
    cursor: &mut Option<JournalSequence>,
    projection: &mut FinalResponseProjection,
    completed: &mut Option<String>,
) -> Result<bool, AppError> {
    let mut changed = false;
    loop {
        let slice = transcript.read_after(*cursor);
        let head = slice.head();
        let entries = slice.into_entries();
        if entries.is_empty() {
            break;
        }
        changed = true;
        for entry in entries {
            *cursor = Some(entry.sequence());
            if let Some(message) = projection.observe(entry.record())? {
                *completed = Some(message);
            }
        }
        if *cursor == head {
            break;
        }
    }
    Ok(changed)
}

fn frame_output(mut message: String) -> String {
    if !message.ends_with('\n') {
        message.push('\n');
    }
    message
}

#[derive(Default)]
struct FinalResponseProjection {
    turn: Option<TurnRef>,
    activities: HashMap<ActivityRef, ProjectedActivity>,
    last_completed_agent_message: Option<String>,
}

struct ProjectedActivity {
    kind: ActivityKind,
    text: String,
}

impl FinalResponseProjection {
    fn observe(&mut self, record: &TranscriptRecord) -> Result<Option<String>, AppError> {
        match record {
            TranscriptRecord::CommandCommitted(AgentCommand::StartTurn { turn, .. }) => {
                if self.turn.replace(*turn).is_some() {
                    return Err(AppError::message(
                        "print Session committed more than one started Turn",
                    ));
                }
            },
            TranscriptRecord::CommandCommitted(AgentCommand::SteerTurn { .. }) => {
                return Err(AppError::message(
                    "print Session committed an unexpected steer Submission",
                ));
            },
            TranscriptRecord::CommandCommitted(_) => {},
            TranscriptRecord::EventCommitted(event) => return self.observe_event(event),
        }
        Ok(None)
    }

    fn observe_event(&mut self, event: &AgentEvent) -> Result<Option<String>, AppError> {
        match event {
            AgentEvent::SessionCreated { .. } | AgentEvent::TurnStarted { .. } => {},
            AgentEvent::ActivityStarted { activity, kind }
                if self.turn == Some(activity.turn()) =>
            {
                if matches!(
                    kind,
                    ActivityKind::ApprovalRequest { .. } | ActivityKind::UserInputRequest { .. }
                ) {
                    return Err(AppError::message(
                        "print Session requires an interactive response",
                    ));
                }
                if self
                    .activities
                    .insert(
                        *activity,
                        ProjectedActivity {
                            kind: *kind,
                            text: String::new(),
                        },
                    )
                    .is_some()
                {
                    return Err(AppError::message(
                        "print Session started the same Activity more than once",
                    ));
                }
            },
            AgentEvent::ActivityStarted { .. } => {},
            AgentEvent::ActivityUpdated { activity, update } => {
                let Some(activity) = self.activities.get_mut(activity) else {
                    return Err(AppError::message(
                        "print Session updated an unknown Activity",
                    ));
                };
                match update {
                    ActivityUpdate::TextDelta(text) => activity.text.push_str(text),
                    ActivityUpdate::TextSnapshot(text) => activity.text.clone_from(text),
                }
            },
            AgentEvent::ActivityFinished { activity, outcome } => {
                let Some(activity) = self.activities.remove(activity) else {
                    return Err(AppError::message(
                        "print Session finished an unknown Activity",
                    ));
                };
                if activity.kind == ActivityKind::AgentMessage
                    && matches!(outcome, ActivityOutcome::Completed)
                {
                    self.last_completed_agent_message = Some(activity.text);
                }
            },
            AgentEvent::TurnFinished { turn, outcome } if self.turn == Some(*turn) => {
                return match outcome {
                    TurnOutcome::Completed => self
                        .last_completed_agent_message
                        .take()
                        .map(Some)
                        .ok_or_else(|| {
                            AppError::message(
                                "print Turn completed without a completed AgentMessage",
                            )
                        }),
                    TurnOutcome::Interrupted => {
                        Err(AppError::message("print Turn was interrupted"))
                    },
                    TurnOutcome::Failed(failure) => Err(AppError::message(format!(
                        "print Turn failed: {}",
                        failure.message()
                    ))),
                };
            },
            AgentEvent::TurnFinished { .. } => {},
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, num::NonZeroU64, path::PathBuf};

    use yo_core::{
        ActivityId, BackendEvent, BackendScriptStep, Failure, HostWorkspacePath, RequestId,
        ScriptedBackend, SessionDescriptor, SubmissionRejection, SubmissionRejectionKind, TurnId,
        WorkspaceHostId, session_repository::LocalSessionRepository,
    };

    use super::*;

    // positional prompt만 있는 TTY, stdin만 있는 pipe, 둘을 함께 쓰는 경우의 순서와 LF
    // 경계를 정확히 보존해 한 Submission 텍스트를 만듭니다.
    #[test]
    fn input_composition_is_stdin_first_and_lf_stable() {
        assert_eq!(
            read_input_from(Some("prompt".to_owned()), &mut &b""[..], true).unwrap(),
            "prompt"
        );
        assert_eq!(
            read_input_from(None, &mut &b"stdin"[..], false).unwrap(),
            "stdin"
        );
        assert_eq!(
            read_input_from(Some("prompt".to_owned()), &mut &b"stdin"[..], false).unwrap(),
            "stdin\nprompt"
        );
        assert_eq!(
            read_input_from(Some("prompt".to_owned()), &mut &b"stdin\n"[..], false).unwrap(),
            "stdin\nprompt"
        );
    }

    // 입력이 없거나 pipe가 UTF-8이 아니면 Session이나 Backend를 만들기 전에 명확히
    // 실패하여 비어 있거나 손상된 Submission을 추측하지 않습니다.
    #[test]
    fn invalid_or_empty_input_fails_before_startup() {
        assert!(read_input_from(None, &mut &b""[..], true).is_err());
        assert!(read_input_from(None, &mut &b""[..], false).is_err());
        assert!(read_input_from(None, &mut &[0xff][..], false).is_err());
    }

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
}
