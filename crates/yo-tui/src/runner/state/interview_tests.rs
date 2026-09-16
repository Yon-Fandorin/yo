use std::{
    env, fs, num,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    thread,
    time::{self, Duration},
};

use yo_core::{
    ActivityApproval, ActivityKind, ActivityRef, ActivityRequestRef, AgentEvent, SubmissionId,
    TranscriptRecord, TurnRef, UserInput, interview,
    interview::{
        AnswerResponse, Capture, InterviewCatalog, InterviewQuestion, InterviewRepository,
        WorkingCopy,
    },
};

use super::{PendingRequest, StateEffect, TuiState};
use crate::{
    appearance,
    input::{
        self,
        event::{InputEvent, KeyAction, KeyCode, KeyModifiers},
    },
    surface::Size,
};
struct Host(InterviewCatalog);
impl crate::InterviewHistoryHost for Host {
    fn resolve(
        &mut self,
        _: ActivityRequestRef,
    ) -> Result<InterviewCatalog, interview::InterviewError> {
        Ok(self.0.clone())
    }
    fn validate_submission(&mut self, _: &WorkingCopy) -> Result<(), interview::InterviewError> {
        Ok(())
    }
}
struct Fixture {
    root: PathBuf,
    catalog: InterviewCatalog,
    copy: WorkingCopy,
}
impl Fixture {
    fn new(submitted: bool) -> Self {
        // macOS의 /var 임시 경로 링크를 저장소의 경로 검증 대상으로 섞지 않는다.
        let temp = fs::canonicalize(env::temp_dir())
            .expect("the TUI interview fixture temp directory must resolve physically");
        let root = temp.join(format!("yo-tui-interview-{}", SubmissionId::new().unwrap()));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let turn = turn();
        let request = ActivityRequestRef::new(
            ActivityRef::new(
                turn,
                yo_core::ActivityId::new(num::NonZeroU64::new(1).unwrap()),
            ),
            yo_core::RequestId::new(num::NonZeroU64::new(1).unwrap()),
        );
        let question = InterviewQuestion {
            id: "q1".into(),
            prompt: "제목".into(),
            question: "질문".into(),
            options: vec![],
            allow_free_text: true,
            allow_notes: true,
            is_secret: false,
        };
        let batch = Capture::batch(request, vec![question.clone()]).unwrap();
        let mut catalog = InterviewCatalog::default();
        let mut record =
            |event| catalog.observe_committed(&TranscriptRecord::EventCommitted(event));
        record(AgentEvent::ActivityStarted {
            activity: request.activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: request.request_id(),
            },
        });
        record(AgentEvent::ActivityUpdated {
            activity: request.activity(),
            update: yo_core::ActivityUpdate::TextSnapshot(batch.to_snapshot().unwrap()),
        });
        if submitted {
            let response = yo_core::ActivityResponse::UserInput(UserInput::new("기존 제출"));
            let answer = question.project_response(&response).unwrap();
            catalog.observe_committed(&TranscriptRecord::CommandCommitted(
                yo_core::AgentCommand::RespondToActivity { request, response },
            ));
            let activity = ActivityRef::new(
                turn,
                yo_core::ActivityId::new(num::NonZeroU64::new(2).unwrap()),
            );
            catalog.observe_committed(&TranscriptRecord::EventCommitted(
                AgentEvent::ActivityStarted {
                    activity,
                    kind: ActivityKind::UserInputResponse {
                        request_id: request.request_id(),
                    },
                },
            ));
            let seal = Capture::AcceptedAnswers {
                interview: request,
                revision: batch.source().1.into(),
                answers: vec![answer],
                answer_responses: vec![AnswerResponse {
                    question_id: "q1".into(),
                    request,
                    response_activity: activity,
                }],
                final_request: request,
                response_activity: activity,
            };
            catalog.observe_committed(&TranscriptRecord::EventCommitted(
                AgentEvent::ActivityUpdated {
                    activity,
                    update: yo_core::ActivityUpdate::TextSnapshot(seal.to_snapshot().unwrap()),
                },
            ));
            catalog.observe_committed(&TranscriptRecord::EventCommitted(
                AgentEvent::ActivityFinished {
                    activity,
                    outcome: yo_core::ActivityOutcome::Completed,
                },
            ));
        }
        let copy = WorkingCopy::new(&catalog.interviews()[0]).unwrap();
        InterviewRepository::open(&root)
            .unwrap()
            .save(&copy, None, &catalog)
            .unwrap();
        Self {
            root,
            catalog,
            copy,
        }
    }
    fn state(&self) -> TuiState {
        let mut state = TuiState::new();
        state.interview = Some(super::super::interview::InterviewController::new(
            InterviewRepository::open(&self.root).unwrap(),
            Box::new(Host(self.catalog.clone())),
        ));
        state
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700));
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn command(state: &mut TuiState, text: &str) -> StateEffect {
    state.restore_draft(text);
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
}

// 복구한 답안에서 Enter는 로컬 편집만 저장하며 실제 Activity 응답이나 Turn을 만들지 않는다.
#[test]
fn recovered_answers_are_saved_without_agent_dispatch() {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    assert_eq!(
        command(
            &mut state,
            &format!("/interview recover {}", fixture.copy.copy_id)
        ),
        StateEffect::Redraw
    );
    state.restore_draft("복구 후 수정한 답안");
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    let saved = InterviewRepository::open(&fixture.root)
        .unwrap()
        .load(&fixture.copy.copy_id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.answers[0].text, "복구 후 수정한 답안");
    assert!(state.active_turn.is_none());
    assert!(state.interview_conversation.is_none());
}

// 다시 연 제출본은 다른 파일로 보관하고 busy 거절 뒤 명시적 send만 새 대화 intent를 만든다.
#[test]
fn reopen_preserves_original_and_busy_send_preserves_copy() {
    let fixture = Fixture::new(true);
    let mut state = fixture.state();
    command(
        &mut state,
        &format!("/interview reopen {}", fixture.copy.copy_id),
    );
    let repo = InterviewRepository::open(&fixture.root).unwrap();
    assert_eq!(repo.list().unwrap().len(), 2);
    assert!(
        repo.load(&fixture.copy.copy_id)
            .unwrap()
            .unwrap()
            .submission
            .is_some()
    );
    state.active_turn = Some(turn());
    assert_eq!(command(&mut state, "/interview send"), StateEffect::Redraw);
    assert!(state.interview_conversation.is_none());
    state.active_turn = None;
    assert_eq!(command(&mut state, "/interview send"), StateEffect::Exit);
    let intent = state.interview_conversation.take().unwrap();
    assert_ne!(intent.copy_id, fixture.copy.copy_id);
    assert_eq!(intent.submission.input().as_str(), intent.preview);

    assert!(intent.preview.contains("기존 제출"));
    assert!(
        repo.load(&fixture.copy.copy_id)
            .unwrap()
            .unwrap()
            .submission
            .is_some()
    );
}

// preview 수정은 입력을 보내지 않고 보관하며 send가 같은 사용자 편집 문자열을 전달한다.
#[test]
fn edited_preview_requires_explicit_new_conversation() {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    command(
        &mut state,
        &format!("/interview recover {}", fixture.copy.copy_id),
    );
    command(&mut state, "/interview preview");
    state.restore_draft("사용자가 편집한 일반 텍스트\n추가 문맥");
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert!(state.interview_conversation.is_none());
    assert_eq!(command(&mut state, "/interview send"), StateEffect::Exit);
    assert_eq!(
        state.interview_conversation.as_ref().unwrap().preview,
        "사용자가 편집한 일반 텍스트\n추가 문맥"
    );
}

// 충돌로 저장하지 못해도 편집 중 답안을 보존하고 게시본은 덮어쓰지 않는다.
#[test]
fn save_conflict_does_not_discard_local_edits() {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    command(
        &mut state,
        &format!("/interview recover {}", fixture.copy.copy_id),
    );
    let repo = InterviewRepository::open(&fixture.root).unwrap();
    let mut winner = fixture.copy.clone();
    winner.context = "다른 writer".into();
    repo.save(&winner, Some(1), &fixture.catalog).unwrap();
    state.restore_draft("내 편집 답안");
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor.text(), "내 편집 답안");
    assert_eq!(
        repo.load(&fixture.copy.copy_id).unwrap().unwrap().context,
        "다른 writer"
    );
}

fn turn() -> TurnRef {
    TurnRef::new(
        "01890f00-0000-7000-8000-000000000001".parse().unwrap(),
        yo_core::TurnId::new(num::NonZeroU64::new(1).unwrap()),
    )
}
fn key(code: KeyCode, modifiers: KeyModifiers) -> InputEvent {
    InputEvent::Key(input::event::KeyEvent {
        code,
        modifiers,
        action: KeyAction::Press,
        state: input::event::KeyState::NONE,
    })
}

// 실제 선택지 이동도 저장하고 아직 제출하지 않은 선택은 제출 완료로 표시하지 않는다.
#[test]
fn live_selection_is_saved_without_submission() {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    let original = &fixture.catalog.interviews()[0];
    let mut question = original.questions[0].clone();
    question.options = (1..=2)
        .map(|index| interview::InterviewOption {
            id: index.to_string(),
            label: format!("선택 {index}"),
            description: String::new(),
        })
        .collect();
    let capture = Capture::batch(original.interview, vec![question]).unwrap();
    let mut durable = InterviewCatalog::default();
    durable.observe_committed(&TranscriptRecord::EventCommitted(
        AgentEvent::ActivityStarted {
            activity: original.interview.activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: original.interview.request_id(),
            },
        },
    ));
    durable.observe_committed(&TranscriptRecord::EventCommitted(
        AgentEvent::ActivityUpdated {
            activity: original.interview.activity(),
            update: yo_core::ActivityUpdate::TextSnapshot(capture.to_snapshot().unwrap()),
        },
    ));
    state.interview = Some(super::super::interview::InterviewController::new(
        InterviewRepository::open(&fixture.root).unwrap(),
        Box::new(Host(durable)),
    ));
    state
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityStarted {
                activity: original.interview.activity(),
                kind: ActivityKind::UserInputRequest {
                    request_id: original.interview.request_id(),
                },
            },
        ))
        .unwrap();
    state
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityUpdated {
                activity: original.interview.activity(),
                update: yo_core::ActivityUpdate::TextSnapshot(capture.to_snapshot().unwrap()),
            },
        ))
        .unwrap();
    let frame = state
        .prepare_frame(
            Size::new(80, 16),
            &appearance::AppearanceState::default().pin(),
        )
        .unwrap();
    state.commit_frame(&frame);
    assert_eq!(
        state
            .handle(key(KeyCode::Down, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert!(state.interview.as_mut().unwrap().flush().is_none());
    let saved = InterviewRepository::open(&fixture.root)
        .unwrap()
        .list()
        .unwrap()
        .into_iter()
        .find(|(id, _)| *id != fixture.copy.copy_id)
        .unwrap()
        .1
        .unwrap();
    assert_eq!(saved.answers[0].option_id.as_deref(), Some("2"));
    assert!(saved.submission.is_none());
    state
        .handle(key(KeyCode::Up, KeyModifiers::ALT), Duration::ZERO)
        .unwrap();
    state.interview.as_mut().unwrap().flush();
    let after_view = InterviewRepository::open(&fixture.root)
        .unwrap()
        .load(&saved.copy_id)
        .unwrap()
        .unwrap();
    assert_eq!(after_view.answers[0].option_id.as_deref(), Some("2"));
}

// 로컬 선택 답안을 Enter로 저장하거나 이동해도 별도로 편집한 notes는 유지한다.
#[test]
fn local_numeric_answer_retains_notes() {
    let fixture = Fixture::new(false);
    let mut question = fixture.catalog.interviews()[0].questions[0].clone();
    question.options = vec![interview::InterviewOption {
        id: "1".into(),
        label: "선택".into(),
        description: String::new(),
    }];
    let capture =
        Capture::batch(fixture.catalog.interviews()[0].interview, vec![question]).unwrap();
    let mut catalog = InterviewCatalog::default();
    catalog.observe_committed(&TranscriptRecord::EventCommitted(
        AgentEvent::ActivityStarted {
            activity: capture.source().0.activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: capture.source().0.request_id(),
            },
        },
    ));
    catalog.observe_committed(&TranscriptRecord::EventCommitted(
        AgentEvent::ActivityUpdated {
            activity: capture.source().0.activity(),
            update: yo_core::ActivityUpdate::TextSnapshot(capture.to_snapshot().unwrap()),
        },
    ));
    let copy = WorkingCopy::new(&catalog.interviews()[0]).unwrap();
    let repo = InterviewRepository::open(&fixture.root).unwrap();
    repo.save(&copy, None, &catalog).unwrap();
    let mut state = fixture.state();
    state.interview = Some(super::super::interview::InterviewController::new(
        repo,
        Box::new(Host(catalog)),
    ));
    command(&mut state, &format!("/interview recover {}", copy.copy_id));
    command(&mut state, "/interview option 1");
    command(&mut state, "/interview notes 사용자 보조 답안");
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    let saved = InterviewRepository::open(&fixture.root)
        .unwrap()
        .load(&copy.copy_id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.answers[0].option_id.as_deref(), Some("1"));
    assert_eq!(saved.answers[0].notes, "사용자 보조 답안");
    command(&mut state, "/interview save");
    state.interview.as_mut().unwrap().flush();
    assert_eq!(
        InterviewRepository::open(&fixture.root)
            .unwrap()
            .load(&copy.copy_id)
            .unwrap()
            .unwrap()
            .answers,
        saved.answers
    );
}

struct UnconfirmedHost(InterviewCatalog);
impl crate::InterviewHistoryHost for UnconfirmedHost {
    fn resolve(
        &mut self,
        _: ActivityRequestRef,
    ) -> Result<InterviewCatalog, interview::InterviewError> {
        Ok(self.0.clone())
    }
    fn validate_submission(&mut self, _: &WorkingCopy) -> Result<(), interview::InterviewError> {
        Err(interview::InterviewError::Invalid(
            "actual Journal receipt is unconfirmed".into(),
        ))
    }
}

// 형식이 맞는 파일만으로 submitted를 주장하지 않고 목록에서도 실제 host 확인 실패를 표시한다.
#[test]
fn stored_submission_without_journal_evidence_is_unavailable() {
    let fixture = Fixture::new(false);
    let mut copy = fixture.copy.clone();
    copy.submission = Some(interview::Submission::NewConversation {
        turn: TurnRef::new(
            "01890f00-0000-7000-8000-000000000002".parse().unwrap(),
            turn().turn_id(),
        ),
        submission_id: SubmissionId::new().unwrap().to_string(),
        accepted_request_sequence: 1,
    });
    let repo = InterviewRepository::open(&fixture.root).unwrap();
    repo.save(&copy, Some(1), &fixture.catalog).unwrap();
    let mut controller = super::super::interview::InterviewController::new(
        repo,
        Box::new(UnconfirmedHost(fixture.catalog.clone())),
    );
    let document = controller.command("list", false).unwrap().document;
    assert!(document.contains("unavailable: actual Journal receipt is unconfirmed"));
    assert!(!document.contains("submitted; use reopen"));
    assert!(
        InterviewRepository::open(&fixture.root)
            .unwrap()
            .load(&copy.copy_id)
            .unwrap()
            .unwrap()
            .submission
            .is_some()
    );
}

// 기존 활성 질문이 있어도 복구한 답안의 Enter는 그 Activity에 응답하지 않는다.
#[test]
fn recovered_copy_does_not_answer_an_outstanding_live_question() {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    let request = ActivityRequestRef::new(
        ActivityRef::new(
            turn(),
            yo_core::ActivityId::new(num::NonZeroU64::new(3).unwrap()),
        ),
        yo_core::RequestId::new(num::NonZeroU64::new(3).unwrap()),
    );
    let mut question = fixture.catalog.interviews()[0].questions[0].clone();
    question.options = vec![interview::InterviewOption {
        id: "1".into(),
        label: "선택".into(),
        description: String::new(),
    }];
    let capture = Capture::batch(request, vec![question]).unwrap();
    state
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityStarted {
                activity: request.activity(),
                kind: ActivityKind::UserInputRequest {
                    request_id: request.request_id(),
                },
            },
        ))
        .unwrap();
    state
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityUpdated {
                activity: request.activity(),
                update: yo_core::ActivityUpdate::TextSnapshot(capture.to_snapshot().unwrap()),
            },
        ))
        .unwrap();
    command(
        &mut state,
        &format!("/interview recover {}", fixture.copy.copy_id),
    );
    assert!(state.request_overlay.is_none());
    assert_eq!(
        state.pending_requests.front(),
        Some(&PendingRequest::UserInput(request))
    );
    state.restore_draft("1");
    let frame = state
        .prepare_frame(
            Size::new(80, 16),
            &appearance::AppearanceState::default().pin(),
        )
        .unwrap();
    state.commit_frame(&frame);
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(
        state.pending_requests.front(),
        Some(&PendingRequest::UserInput(request))
    );
    assert!(state.interview_conversation.is_none());
}

// 복구 편집 중 후속 질문 단축키와 기존 대기열이 새 모델 입력을 만들지 않는다.
#[test]
fn recovered_editor_disables_follow_up_shortcuts_and_release() {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    command(
        &mut state,
        &format!("/interview recover {}", fixture.copy.copy_id),
    );
    state.restore_draft("편집 중인 답안");
    state
        .handle(
            key(KeyCode::Character('q'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    assert!(state.follow_ups.is_empty());
    assert_eq!(state.editor.text(), "편집 중인 답안");
    state.follow_ups.push_back(UserInput::new("기존 후속 질문"));
    assert!(state.next_follow_up().unwrap().is_none());
    assert!(state.pending_submissions.is_empty());
}

// 편집 중 새 승인이 도착해도 Enter를 승인으로 넘기거나 로컬 답안 저장을 막지 않는다.
#[test]
fn recovered_editor_keeps_new_approval_separate() {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    command(
        &mut state,
        &format!("/interview recover {}", fixture.copy.copy_id),
    );
    let request = ActivityRequestRef::new(
        ActivityRef::new(
            turn(),
            yo_core::ActivityId::new(num::NonZeroU64::new(3).unwrap()),
        ),
        yo_core::RequestId::new(num::NonZeroU64::new(3).unwrap()),
    );
    state
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityStarted {
                activity: request.activity(),
                kind: ActivityKind::ApprovalRequest {
                    request_id: request.request_id(),
                },
            },
        ))
        .unwrap();
    let profile = ActivityApproval {
        related_change: None,
        plain_text: "실행 승인".into(),
        decline_choice: Some(2),
        choices: vec![
            yo_core::ApprovalChoice {
                label: "허용".into(),
                description: String::new(),
                enabled: true,
            },
            yo_core::ApprovalChoice {
                label: "거절".into(),
                description: String::new(),
                enabled: true,
            },
        ],
    };
    state
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityUpdated {
                activity: request.activity(),
                update: yo_core::ActivityUpdate::TextSnapshot(profile.to_snapshot().unwrap()),
            },
        ))
        .unwrap();
    assert!(state.request_overlay.is_none());
    state.restore_draft("로컬 답안");
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(
        state.pending_requests.front(),
        Some(&PendingRequest::Approval(request))
    );
    assert_eq!(
        InterviewRepository::open(&fixture.root)
            .unwrap()
            .load(&fixture.copy.copy_id)
            .unwrap()
            .unwrap()
            .answers[0]
            .text,
        "로컬 답안"
    );
}

// 새 Session의 실제 durable 수락만 제출 표시를 만들고 저장 실패 뒤 명시적 save가 같은 기록을
// 게시한다.
#[test]
fn new_conversation_receipt_survives_save_failure_without_replaying_input() {
    use yo_core::{
        AgentCommand, AgentIntent, AgentSession, BackendBindingEvidence, BackendCommandEvidence,
        BackendIdentity, BackendRequestEvidence, BackendScriptStep, ContinuationStrategy,
        HostWorkspacePath, ScriptedBackend, SessionDescriptor, TurnId,
        session_repository::LocalSessionRepository,
    };
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    command(
        &mut state,
        &format!("/interview recover {}", fixture.copy.copy_id),
    );
    let controller = state.interview.as_mut().unwrap();
    let another = ActivityRequestRef::new(
        ActivityRef::new(
            turn(),
            yo_core::ActivityId::new(num::NonZeroU64::new(3).unwrap()),
        ),
        yo_core::RequestId::new(num::NonZeroU64::new(3).unwrap()),
    );
    let capture =
        Capture::batch(another, fixture.catalog.interviews()[0].questions.clone()).unwrap();
    controller.observe(&TranscriptRecord::EventCommitted(
        AgentEvent::ActivityStarted {
            activity: another.activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: another.request_id(),
            },
        },
    ));
    controller.observe(&TranscriptRecord::EventCommitted(
        AgentEvent::ActivityUpdated {
            activity: another.activity(),
            update: yo_core::ActivityUpdate::TextSnapshot(capture.to_snapshot().unwrap()),
        },
    ));
    let intent = controller
        .command("send", false)
        .unwrap()
        .conversation
        .unwrap();
    let descriptor = SessionDescriptor::new(
        "10000000-0000-4000-8000-000000000001".parse().unwrap(),
        HostWorkspacePath::normalize_local(&fixture.root).unwrap(),
    )
    .unwrap();
    let session_id = descriptor.session_id();
    let new_turn = TurnRef::new(session_id, TurnId::new(num::NonZeroU64::new(1).unwrap()));
    let binding = BackendBindingEvidence::new(
        "fixture",
        "1",
        BackendIdentity::new("fixture.binding/v1", "binding"),
        BackendIdentity::new("fixture.model/v1", "model"),
        BackendIdentity::new("fixture.session/v1", "session"),
        ContinuationStrategy::BackendManagedState,
    );
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::CreateSession { session_id },
            evidence: BackendCommandEvidence::BindingOpened(binding),
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::StartTurn {
                turn: new_turn,
                input: intent.submission.input().clone(),
            },
            evidence: BackendCommandEvidence::RequestAccepted(BackendRequestEvidence::new(
                "fixture.request/v1",
                BackendIdentity::new("fixture.exchange/v1", "exchange"),
                BackendIdentity::new("fixture.accepted/v1", "accepted"),
            )),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let repository =
        LocalSessionRepository::open(fixture.root.join("sessions"), 16 * 1024 * 1024).unwrap();
    let mut session =
        AgentSession::start_cancellable_with_repository(backend, descriptor, repository, || false)
            .unwrap()
            .unwrap();
    let reader = session.transcript_reader();
    controller.accepted_pending(intent.clone(), reader.clone());
    for entry in reader.read_after(None).entries() {
        controller.observe(entry.record());
    }
    assert_eq!(
        InterviewRepository::open(&fixture.root)
            .unwrap()
            .list()
            .unwrap()
            .len(),
        1
    );
    assert!(controller.tick().is_none());
    assert!(
        InterviewRepository::open(&fixture.root)
            .unwrap()
            .load(&fixture.copy.copy_id)
            .unwrap()
            .unwrap()
            .submission
            .is_none()
    );
    assert!(matches!(
        session
            .dispatch(AgentIntent::Submit(intent.submission.clone()))
            .unwrap(),
        yo_core::CommandAdmission::Queued
    ));
    let deadline = time::Instant::now() + Duration::from_secs(2);
    let (accepted, sequence) = loop {
        if let Some(receipt) = reader.accepted_initial_submission(intent.submission.id()) {
            break receipt;
        }
        assert!(time::Instant::now() < deadline);
        thread::yield_now();
    };
    fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o500)).unwrap();
    assert!(controller.tick().unwrap().contains("Unsaved"));
    fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        InterviewRepository::open(&fixture.root)
            .unwrap()
            .load(&fixture.copy.copy_id)
            .unwrap()
            .unwrap()
            .submission
            .is_none()
    );
    controller.command("save", false).unwrap();
    let saved = InterviewRepository::open(&fixture.root)
        .unwrap()
        .load(&fixture.copy.copy_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.submission,
        Some(interview::Submission::NewConversation {
            turn: accepted,
            submission_id: intent.submission.id().to_string(),
            accepted_request_sequence: sequence.get()
        })
    );
    assert_eq!(saved.generation, 2);
    assert_ne!(
        accepted.session_id(),
        fixture.copy.source().0.activity().turn().session_id()
    );
    session.shutdown().unwrap();
}

// 원본 Journal이 volatile/unavailable이면 진짜 질문도 durable Saved 파일을 새로 만들지 않는다.
#[test]
fn volatile_capture_cannot_claim_a_saved_recoverable_copy() {
    struct Unavailable;
    impl crate::InterviewHistoryHost for Unavailable {
        fn resolve(
            &mut self,
            _: ActivityRequestRef,
        ) -> Result<InterviewCatalog, interview::InterviewError> {
            Err(interview::InterviewError::Invalid(
                "volatile Journal".into(),
            ))
        }
        fn validate_submission(
            &mut self,
            _: &WorkingCopy,
        ) -> Result<(), interview::InterviewError> {
            Ok(())
        }
    }
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    state.interview = Some(super::super::interview::InterviewController::new(
        InterviewRepository::open(&fixture.root).unwrap(),
        Box::new(Unavailable),
    ));
    let original = &fixture.catalog.interviews()[0];
    let capture = Capture::batch(original.interview, original.questions.clone()).unwrap();
    for event in [
        AgentEvent::ActivityStarted {
            activity: original.interview.activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: original.interview.request_id(),
            },
        },
        AgentEvent::ActivityUpdated {
            activity: original.interview.activity(),
            update: yo_core::ActivityUpdate::TextSnapshot(capture.to_snapshot().unwrap()),
        },
    ] {
        state
            .observe_record(TranscriptRecord::EventCommitted(event))
            .unwrap();
    }
    assert_eq!(
        InterviewRepository::open(&fixture.root)
            .unwrap()
            .list()
            .unwrap()
            .len(),
        1
    );
    let document = state
        .interview
        .as_mut()
        .unwrap()
        .command("save", false)
        .unwrap()
        .document;
    assert!(document.contains("Volatile") && document.contains("unavailable"));
}

// 임시 저장 오류 뒤 새 편집은 1초 안에 다시 시도하며 명시적 save 없이 최신 답변을 게시한다.
#[test]
fn new_edits_reenable_autosave_after_transient_io_error() {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    command(
        &mut state,
        &format!("/interview recover {}", fixture.copy.copy_id),
    );
    let controller = state.interview.as_mut().unwrap();
    fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o500)).unwrap();
    controller.edit_text("첫 편집", None, None);
    assert!(controller.flush().is_some());
    fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o700)).unwrap();
    controller.edit_text("다음 편집", None, None);
    thread::sleep(Duration::from_millis(300));
    controller.tick();
    assert_eq!(
        InterviewRepository::open(&fixture.root)
            .unwrap()
            .load(&fixture.copy.copy_id)
            .unwrap()
            .unwrap()
            .answers[0]
            .text,
        "다음 편집"
    );
}

// 명령 이름으로 시작하는 답변은 // escape로 정확히 저장하고 다시 보여도 명령으로 실행하지 않는다.
#[test]
fn slash_prefixed_literal_answers_can_be_saved_and_recovered() {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    command(
        &mut state,
        &format!("/interview recover {}", fixture.copy.copy_id),
    );
    assert_eq!(
        command(&mut state, "//interview is the command name"),
        StateEffect::Redraw
    );
    assert_eq!(
        InterviewRepository::open(&fixture.root)
            .unwrap()
            .load(&fixture.copy.copy_id)
            .unwrap()
            .unwrap()
            .answers[0]
            .text,
        "/interview is the command name"
    );
    assert_eq!(state.editor.text(), "//interview is the command name");
    assert_eq!(
        command(&mut state, "//interview is the command name"),
        StateEffect::Redraw
    );
    assert!(state.interview_conversation.is_none());
}

// 실제 durable Session이 최초 요청 receipt 없이 끝나면 intent는 유지하되 running 앱에서 편집한다.
#[test]
fn terminal_without_acceptance_restores_editable_copy_and_exact_intent() {
    use yo_core::{
        AgentCommand, AgentIntent, AgentSession, BackendEvent, BackendScriptStep,
        HostWorkspacePath, ScriptedBackend, SessionDescriptor, TurnId, TurnOutcome,
        session_repository::LocalSessionRepository,
    };
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    command(
        &mut state,
        &format!("/interview recover {}", fixture.copy.copy_id),
    );
    let intent = state
        .interview
        .as_mut()
        .unwrap()
        .command("send", false)
        .unwrap()
        .conversation
        .unwrap();
    let descriptor = SessionDescriptor::new(
        "10000000-0000-4000-8000-000000000002".parse().unwrap(),
        HostWorkspacePath::normalize_local(&fixture.root).unwrap(),
    )
    .unwrap();
    let session_id = descriptor.session_id();
    let new_turn = TurnRef::new(session_id, TurnId::new(num::NonZeroU64::new(1).unwrap()));
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession { session_id }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: new_turn,
            input: intent.submission.input().clone(),
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: new_turn,
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let repo =
        LocalSessionRepository::open(fixture.root.join("terminal-session"), 16 * 1024 * 1024)
            .unwrap();
    let mut session =
        AgentSession::start_cancellable_with_repository(backend, descriptor, repo, || false)
            .unwrap()
            .unwrap();
    let reader = session.transcript_reader();
    state
        .interview
        .as_mut()
        .unwrap()
        .accepted_pending(intent.clone(), reader.clone());
    assert!(state.interview.as_mut().unwrap().tick().is_none());
    session
        .dispatch(AgentIntent::Submit(intent.submission.clone()))
        .unwrap();
    let deadline = time::Instant::now() + Duration::from_secs(2);
    while !reader.initial_submission_terminated(intent.submission.id()) {
        assert!(time::Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(
        reader
            .accepted_initial_submission(intent.submission.id())
            .is_none()
    );
    assert!(state.tick_interview().unwrap());
    assert!(state.interview.as_ref().unwrap().is_editing());
    assert_eq!(state.editor.text(), intent.preview);
    assert!(
        format!("{:?}", state.interview.as_ref().unwrap())
            .contains(&intent.submission.id().to_string())
    );
    command(&mut state, "/interview next");
    assert_eq!(command(&mut state, "실패 후 수정"), StateEffect::Redraw);
    let saved = InterviewRepository::open(&fixture.root)
        .unwrap()
        .load(&fixture.copy.copy_id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.answers[0].text, "실패 후 수정");
    assert!(saved.submission.is_none());
    assert!(state.interview_conversation.is_none());
    session.shutdown().unwrap();
}

// 한 글자씩 입력한 명령 prefix는 답변이 아니며 save가 중간 /intervie를 게시하지 않는다.
#[test]
fn typed_interview_command_prefix_does_not_overwrite_answer() {
    let fixture = Fixture::new(false);
    let mut state = fixture.state();
    command(
        &mut state,
        &format!("/interview recover {}", fixture.copy.copy_id),
    );
    for character in "/interview save".chars() {
        assert_eq!(
            state
                .handle(
                    key(KeyCode::Character(character), KeyModifiers::NONE),
                    Duration::ZERO
                )
                .unwrap(),
            StateEffect::Redraw
        );
    }
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    assert_eq!(
        InterviewRepository::open(&fixture.root)
            .unwrap()
            .load(&fixture.copy.copy_id)
            .unwrap()
            .unwrap()
            .answers[0]
            .text,
        ""
    );
    assert!(state.interview_conversation.is_none());
}
