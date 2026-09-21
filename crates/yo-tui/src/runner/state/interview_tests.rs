use std::{env, fs, num, os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};

use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityUpdate, AgentEvent,
    SessionId, SubmissionId, TranscriptRecord, TurnRef, interview,
    interview::{Capture, InterviewCatalog, InterviewQuestion, InterviewRepository, WorkingCopy},
};

use super::{StateEffect, TuiState};
use crate::input::event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState};

struct Host(InterviewCatalog);
impl crate::InterviewHistoryHost for Host {
    fn resolve(
        &mut self,
        _: ActivityRequestRef,
    ) -> Result<InterviewCatalog, interview::InterviewError> {
        Ok(self.0.clone())
    }

    fn historical_interviews(
        &mut self,
        _: SessionId,
    ) -> Result<InterviewCatalog, interview::InterviewError> {
        Ok(self.0.clone())
    }
}

struct Fixture {
    root: PathBuf,
    catalog: InterviewCatalog,
    copy: WorkingCopy,
    request: ActivityRequestRef,
    batch: Capture,
}

fn turn() -> TurnRef {
    TurnRef::new(
        "01890f00-0000-7000-8000-000000000001".parse().unwrap(),
        yo_core::TurnId::new(num::NonZeroU64::new(1).unwrap()),
    )
}

impl Fixture {
    fn new(submitted: bool) -> Self {
        let temp = fs::canonicalize(env::temp_dir()).unwrap();
        let root = temp.join(format!(
            "yo-contextual-draft-{}",
            SubmissionId::new().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let request = ActivityRequestRef::new(
            ActivityRef::new(
                turn(),
                yo_core::ActivityId::new(num::NonZeroU64::new(1).unwrap()),
            ),
            yo_core::RequestId::new(num::NonZeroU64::new(1).unwrap()),
        );
        let question = InterviewQuestion {
            id: "q1".into(),
            prompt: "Public question".into(),
            question: "What is the answer?".into(),
            options: vec![],
            allow_free_text: true,
            allow_notes: true,
            is_secret: false,
        };
        let batch = Capture::batch(request, vec![question.clone()]).unwrap();
        let mut catalog = InterviewCatalog::default();
        for event in [
            AgentEvent::ActivityStarted {
                activity: request.activity(),
                kind: ActivityKind::UserInputRequest {
                    request_id: request.request_id(),
                },
            },
            AgentEvent::ActivityUpdated {
                activity: request.activity(),
                update: ActivityUpdate::TextSnapshot(batch.to_snapshot().unwrap()),
            },
        ] {
            catalog.observe_committed(&TranscriptRecord::EventCommitted(event));
        }
        let copy = WorkingCopy::new_contextual(&catalog.interviews()[0]).unwrap();
        if submitted {
            let response = yo_core::ActivityResponse::UserInput("sealed answer".into());
            let answer = question.project_response(&response).unwrap();
            catalog.observe_committed(&TranscriptRecord::CommandCommitted(
                yo_core::AgentCommand::RespondToActivity { request, response },
            ));
            let activity = ActivityRef::new(
                turn(),
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
                answer_responses: vec![interview::AnswerResponse {
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
                    update: ActivityUpdate::TextSnapshot(seal.to_snapshot().unwrap()),
                },
            ));
            catalog.observe_committed(&TranscriptRecord::EventCommitted(
                AgentEvent::ActivityFinished {
                    activity,
                    outcome: ActivityOutcome::Completed,
                },
            ));
        }
        InterviewRepository::open(&root)
            .unwrap()
            .save(&copy, None, &catalog)
            .unwrap();
        Self {
            root,
            catalog,
            copy,
            request,
            batch,
        }
    }

    fn controller(&self, session: SessionId) -> super::super::interview::InterviewController {
        self.controller_with_resume(session, false)
    }

    fn controller_with_resume(
        &self,
        session: SessionId,
        is_resume: bool,
    ) -> super::super::interview::InterviewController {
        super::super::interview::InterviewController::new(
            InterviewRepository::open(&self.root).unwrap(),
            Box::new(Host(self.catalog.clone())),
            session,
            is_resume,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// 종료된 요청은 읽기만 허용하고 보관함 및 새 대화 명령을 노출하지 않는다.
#[test]
fn dead_draft_is_contextual_read_only_and_has_no_archive_commands() {
    let fixture = Fixture::new(false);
    let mut controller = fixture.controller(turn().session_id());
    assert!(controller.tick().unwrap().contains("/interview view"));
    let offer = controller.command("").unwrap().document;
    assert!(offer.contains("/interview view"));
    assert!(offer.contains("/interview discard"));
    assert!(!offer.contains(&fixture.copy.copy_id));
    assert!(!offer.contains("/interview send"));
    let view = controller.command("view").unwrap();
    assert!(!view.document.contains("sealed"));
    assert!(view.editor.is_none());
    assert!(controller.command("continue").is_err());
    for obsolete in ["list", "recover", "reopen", "preview", "send"] {
        assert!(controller.command(obsolete).is_err());
    }
}

// 선택하지 않은 세션의 초안은 발견하거나 변경하지 않는다.
#[test]
fn other_session_cannot_discover_draft() {
    let fixture = Fixture::new(false);
    let other: SessionId = "01890f00-0000-7000-8000-000000000002".parse().unwrap();
    let mut controller = fixture.controller(other);
    assert!(
        controller
            .command("")
            .unwrap()
            .document
            .contains("No unfinished")
    );
    assert!(
        InterviewRepository::open(&fixture.root)
            .unwrap()
            .load(&fixture.copy.copy_id)
            .unwrap()
            .is_some()
    );
}

// 재개 시 기록으로 재생된 과거 요청은 완료 이벤트가 없어도 살아 있다고 간주하지 않는다.
#[test]
fn replayed_request_after_resume_remains_read_only() {
    let fixture = Fixture::new(false);
    let mut controller = fixture.controller_with_resume(turn().session_id(), true);
    for event in [
        AgentEvent::ActivityStarted {
            activity: fixture.request.activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: fixture.request.request_id(),
            },
        },
        AgentEvent::ActivityUpdated {
            activity: fixture.request.activity(),
            update: ActivityUpdate::TextSnapshot(fixture.batch.to_snapshot().unwrap()),
        },
    ] {
        controller.observe(&TranscriptRecord::EventCommitted(event));
    }
    assert!(controller.command("continue").is_err());
    assert!(controller.command("view").is_ok());
}

// 손상된 보관 파일이 있으면 다른 초안을 선택하거나 새로 만들지 않고 진단한다.
#[test]
fn malformed_record_blocks_contextual_selection_without_mutation() {
    let fixture = Fixture::new(false);
    let malformed = fixture
        .root
        .join("00000000-0000-4000-8000-0000000000aa.json");
    fs::write(&malformed, b"{broken").unwrap();
    fs::set_permissions(&malformed, fs::Permissions::from_mode(0o600)).unwrap();
    let mut controller = fixture.controller(turn().session_id());
    assert!(controller.command("").is_err());
    for event in [
        AgentEvent::ActivityStarted {
            activity: fixture.request.activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: fixture.request.request_id(),
            },
        },
        AgentEvent::ActivityUpdated {
            activity: fixture.request.activity(),
            update: ActivityUpdate::TextSnapshot(fixture.batch.to_snapshot().unwrap()),
        },
    ] {
        controller.observe(&TranscriptRecord::EventCommitted(event));
    }
    assert!(controller.command("continue").is_err());
    assert_eq!(fs::read(&malformed).unwrap(), b"{broken");
    assert!(
        InterviewRepository::open(&fixture.root)
            .unwrap()
            .load(&fixture.copy.copy_id)
            .unwrap()
            .is_some()
    );
}

// 명시적 폐기는 현재 세션의 초안과 안내를 함께 제거한다.
#[test]
fn explicit_discard_deletes_the_contextual_draft() {
    let fixture = Fixture::new(false);
    let repository = InterviewRepository::open(&fixture.root).unwrap();
    let mut controller = fixture.controller(turn().session_id());
    controller.command("discard").unwrap();
    assert!(repository.load(&fixture.copy.copy_id).unwrap().is_none());
    assert!(
        controller
            .command("")
            .unwrap()
            .document
            .contains("No unfinished")
    );
}

// 동일한 요청에 초안이 둘이면 임의 선택이나 삭제 없이 오류로 멈춘다.
#[test]
fn duplicate_contextual_drafts_fail_closed_without_deleting_either() {
    let fixture = Fixture::new(false);
    let second = WorkingCopy::new_contextual(&fixture.catalog.interviews()[0]).unwrap();
    let repository = InterviewRepository::open(&fixture.root).unwrap();
    repository.save(&second, None, &fixture.catalog).unwrap();
    let mut controller = fixture.controller(turn().session_id());
    assert!(controller.command("continue").is_err());
    assert!(controller.command("discard").is_err());
    assert!(repository.load(&fixture.copy.copy_id).unwrap().is_some());
    assert!(repository.load(&second.copy_id).unwrap().is_some());
}

// 이전 형식의 사본은 새 UI에서 숨기되 디스크에는 그대로 둔다.
#[test]
fn older_working_copies_are_left_untouched_and_unlisted() {
    let fixture = Fixture::new(false);
    let old = WorkingCopy::new(&fixture.catalog.interviews()[0]).unwrap();
    let old_v4 = WorkingCopy::new_contextual(&fixture.catalog.interviews()[0]).unwrap();
    let old_v4 = WorkingCopy::decode(
        String::from_utf8(old_v4.encode().unwrap())
            .unwrap()
            .replacen("yo.interview-draft/v1", "yo.interview-working-copy/v4", 1)
            .as_bytes(),
    )
    .unwrap();
    let repository = InterviewRepository::open(&fixture.root).unwrap();
    repository.save(&old, None, &fixture.catalog).unwrap();
    repository.save(&old_v4, None, &fixture.catalog).unwrap();
    let mut controller = fixture.controller(turn().session_id());
    controller.command("discard").unwrap();
    assert!(controller.command("send").is_err());
    assert!(
        controller
            .command("")
            .unwrap()
            .document
            .contains("No unfinished")
    );
    assert!(repository.load(&old.copy_id).unwrap().is_some());
    assert!(repository.load(&old_v4.copy_id).unwrap().is_some());
}

// 최종 응답의 영속 확인 후 남은 초안을 다시 표시하지 않고 제거한다.
#[test]
fn durable_final_seal_removes_leftover_draft_before_presentation() {
    let fixture = Fixture::new(true);
    let mut controller = fixture.controller(turn().session_id());
    assert!(
        controller
            .command("")
            .unwrap()
            .document
            .contains("No unfinished")
    );
    assert!(
        InterviewRepository::open(&fixture.root)
            .unwrap()
            .load(&fixture.copy.copy_id)
            .unwrap()
            .is_none()
    );
}

// 최종 봉인 뒤 늦게 들어온 활동 이벤트가 제출된 답변을 새 초안으로 만들지 않는다.
#[test]
fn submitted_capture_does_not_recreate_draft_after_cleanup() {
    let fixture = Fixture::new(true);
    let repository = InterviewRepository::open(&fixture.root).unwrap();
    repository.delete(&fixture.copy).unwrap();
    let mut controller = fixture.controller(turn().session_id());
    for event in [
        AgentEvent::ActivityStarted {
            activity: fixture.request.activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: fixture.request.request_id(),
            },
        },
        AgentEvent::ActivityUpdated {
            activity: fixture.request.activity(),
            update: ActivityUpdate::TextSnapshot(fixture.batch.to_snapshot().unwrap()),
        },
    ] {
        assert!(
            controller
                .observe(&TranscriptRecord::EventCommitted(event))
                .is_none()
        );
    }
    assert!(repository.list().unwrap().is_empty());
    assert!(
        controller
            .command("")
            .unwrap()
            .document
            .contains("No unfinished")
    );
}

// 살아 있는 정확한 활동의 초안만 현재 요청으로 이어 쓸 수 있다.
#[test]
fn live_request_can_restore_draft_only_for_its_exact_activity() {
    let fixture = Fixture::new(false);
    let mut controller = fixture.controller(turn().session_id());
    for event in [
        AgentEvent::ActivityStarted {
            activity: fixture.request.activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: fixture.request.request_id(),
            },
        },
        AgentEvent::ActivityUpdated {
            activity: fixture.request.activity(),
            update: ActivityUpdate::TextSnapshot(fixture.batch.to_snapshot().unwrap()),
        },
    ] {
        let notice = controller.observe(&TranscriptRecord::EventCommitted(event));
        if let Some(notice) = notice {
            assert!(notice.contains("/interview continue"));
        }
    }
    let offer = controller.command("").unwrap().document;
    assert!(offer.contains("/interview continue"));
    assert!(!offer.contains("/interview view"));
    let continuation = controller.command("continue").unwrap();
    assert!(continuation.editor.is_some());
    controller.observe(&TranscriptRecord::EventCommitted(
        AgentEvent::ActivityFinished {
            activity: fixture.request.activity(),
            outcome: ActivityOutcome::Interrupted,
        },
    ));
    assert!(controller.command("continue").is_err());
    assert!(controller.command("view").is_ok());
}

// 같은 세션에 종료된 초안이 남아 있어도 현재 살아 있는 요청을 먼저 이어 쓴다.
#[test]
fn live_draft_takes_priority_over_an_older_dead_draft() {
    let fixture = Fixture::new(false);
    let repository = InterviewRepository::open(&fixture.root).unwrap();
    repository.delete(&fixture.copy).unwrap();
    let mut older = fixture.copy.clone();
    older.copy_id = "00000000-0000-4000-8000-000000000001".into();
    repository.save(&older, None, &fixture.catalog).unwrap();

    let request = ActivityRequestRef::new(
        ActivityRef::new(
            turn(),
            yo_core::ActivityId::new(num::NonZeroU64::new(3).unwrap()),
        ),
        yo_core::RequestId::new(num::NonZeroU64::new(2).unwrap()),
    );
    let batch = Capture::batch(
        request,
        vec![fixture.catalog.interviews()[0].questions[0].clone()],
    )
    .unwrap();
    let started = AgentEvent::ActivityStarted {
        activity: request.activity(),
        kind: ActivityKind::UserInputRequest {
            request_id: request.request_id(),
        },
    };
    let updated = AgentEvent::ActivityUpdated {
        activity: request.activity(),
        update: ActivityUpdate::TextSnapshot(batch.to_snapshot().unwrap()),
    };
    let mut catalog = fixture.catalog.clone();
    for event in [started.clone(), updated.clone()] {
        catalog.observe_committed(&TranscriptRecord::EventCommitted(event));
    }
    let mut newer = WorkingCopy::new_contextual(
        catalog
            .interviews()
            .iter()
            .find(|capture| capture.interview == request)
            .unwrap(),
    )
    .unwrap();
    newer.copy_id = "00000000-0000-4000-8000-000000000002".into();
    repository.save(&newer, None, &catalog).unwrap();

    let mut controller = super::super::interview::InterviewController::new(
        repository,
        Box::new(Host(catalog)),
        turn().session_id(),
        false,
    );
    for event in [started, updated] {
        controller.observe(&TranscriptRecord::EventCommitted(event));
    }
    let continuation = controller.command("continue").unwrap();
    assert!(continuation.editor.is_some());
    assert!(continuation.document.contains("Public question"));
}

// 폐기 이후 늦게 도착한 활동 갱신으로 초안을 다시 만들지 않는다.
#[test]
fn discarding_a_live_draft_does_not_recreate_it_on_later_activity() {
    let fixture = Fixture::new(false);
    let mut controller = fixture.controller(turn().session_id());
    for event in [
        AgentEvent::ActivityStarted {
            activity: fixture.request.activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: fixture.request.request_id(),
            },
        },
        AgentEvent::ActivityUpdated {
            activity: fixture.request.activity(),
            update: ActivityUpdate::TextSnapshot(fixture.batch.to_snapshot().unwrap()),
        },
    ] {
        controller.observe(&TranscriptRecord::EventCommitted(event));
    }
    controller.command("discard").unwrap();
    controller.observe(&TranscriptRecord::EventCommitted(
        AgentEvent::ActivityUpdated {
            activity: fixture.request.activity(),
            update: ActivityUpdate::TextSnapshot(fixture.batch.to_snapshot().unwrap()),
        },
    ));
    assert!(
        InterviewRepository::open(&fixture.root)
            .unwrap()
            .load(&fixture.copy.copy_id)
            .unwrap()
            .is_none()
    );
    assert!(
        controller
            .command("")
            .unwrap()
            .document
            .contains("No unfinished")
    );
}

// TUI 명령은 현재 세션에 머물며 별도의 대화를 시작하지 않는다.
#[test]
fn tui_command_stays_in_session_and_never_starts_new_conversation() {
    let fixture = Fixture::new(false);
    let mut state = TuiState::new();
    state.interview = Some(fixture.controller(turn().session_id()));
    state.restore_draft("/interview");
    let effect = state
        .handle(
            InputEvent::Key(KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                action: KeyAction::Press,
                state: KeyState::NONE,
            }),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(effect, StateEffect::Redraw);
    state.restore_draft("/interview send");
    let send_effect = state
        .handle(
            InputEvent::Key(KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                action: KeyAction::Press,
                state: KeyState::NONE,
            }),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(send_effect, StateEffect::Redraw);
    assert!(state.pending_submissions.is_empty());
}
