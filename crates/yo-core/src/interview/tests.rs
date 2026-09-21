#[cfg(test)]
use std::env;
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::fs::Permissions;
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
#[cfg(test)]
use std::sync::atomic::Ordering;
#[cfg(test)]
use std::thread;
#[cfg(test)]
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;
#[cfg(test)]
use std::time::SystemTime;
use std::{
    num::NonZeroU64,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
};

use super::*;
#[cfg(test)]
use crate::journal::SessionJournal;
#[cfg(test)]
use crate::session_repository::DurableCutoff;
#[cfg(test)]
use crate::session_repository::RepositorySequence;
use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityResponse,
    ActivityUpdate, AgentCommand, AgentEvent, RequestId, TranscriptRecord, TurnId, TurnRef,
    UserInput,
};

fn request(index: u64) -> ActivityRequestRef {
    ActivityRequestRef::new(
        ActivityRef::new(
            TurnRef::new(
                crate::fixture_session(1),
                TurnId::new(NonZeroU64::new(1).unwrap()),
            ),
            ActivityId::new(NonZeroU64::new(index).unwrap()),
        ),
        RequestId::new(NonZeroU64::new(index).unwrap()),
    )
}
fn questions() -> Vec<InterviewQuestion> {
    (1..=2)
        .map(|i| InterviewQuestion {
            id: format!("q{i}"),
            prompt: format!("질문 {i}"),
            question: "어떤 답인가요?".into(),
            options: vec![InterviewOption {
                id: "1".into(),
                label: "선택".into(),
                description: "설명".into(),
            }],
            allow_free_text: true,
            allow_notes: true,
            is_secret: false,
        })
        .collect()
}
fn secret_questions() -> Vec<InterviewQuestion> {
    let mut questions = questions();
    questions[1].options.clear();
    questions[1].allow_notes = false;
    questions[1].is_secret = true;
    questions
}
fn event(catalog: &mut InterviewCatalog, event: AgentEvent) {
    catalog.observe_committed(&TranscriptRecord::EventCommitted(event));
}
fn batch() -> (InterviewCatalog, Capture) {
    let capture = Capture::batch(request(1), questions()).unwrap();
    let mut catalog = InterviewCatalog::default();
    event(
        &mut catalog,
        AgentEvent::ActivityStarted {
            activity: request(1).activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: request(1).request_id(),
            },
        },
    );
    event(
        &mut catalog,
        AgentEvent::ActivityUpdated {
            activity: request(1).activity(),
            update: ActivityUpdate::TextSnapshot(capture.to_snapshot().unwrap()),
        },
    );
    (catalog, capture)
}

fn catalog_for(
    interview: ActivityRequestRef,
    questions: Vec<InterviewQuestion>,
) -> InterviewCatalog {
    let capture = Capture::batch(interview, questions).unwrap();
    let mut catalog = InterviewCatalog::default();
    event(
        &mut catalog,
        AgentEvent::ActivityStarted {
            activity: interview.activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: interview.request_id(),
            },
        },
    );
    event(
        &mut catalog,
        AgentEvent::ActivityUpdated {
            activity: interview.activity(),
            update: ActivityUpdate::TextSnapshot(capture.to_snapshot().unwrap()),
        },
    );
    catalog
}
fn answered(
    catalog: &mut InterviewCatalog,
    req: ActivityRequestRef,
    q: usize,
    value: &str,
    completed: bool,
) -> (Answer, AnswerResponse) {
    let response = ActivityResponse::UserInput(UserInput::new(value));
    let answer = questions()[q].project_response(&response).unwrap();
    catalog.observe_committed(&TranscriptRecord::CommandCommitted(
        AgentCommand::RespondToActivity {
            request: req,
            response,
        },
    ));
    let activity = request(req.activity().activity_id().get().get() + 10).activity();
    event(
        catalog,
        AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::UserInputResponse {
                request_id: req.request_id(),
            },
        },
    );
    if completed {
        event(
            catalog,
            AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            },
        );
    }
    (
        answer,
        AnswerResponse {
            question_id: format!("q{}", q + 1),
            request: req,
            response_activity: activity,
        },
    )
}
fn next_question(
    catalog: &mut InterviewCatalog,
    capture: &Capture,
    req: ActivityRequestRef,
    index: usize,
) {
    let (interview, revision) = capture.source();
    event(
        catalog,
        AgentEvent::ActivityStarted {
            activity: req.activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: req.request_id(),
            },
        },
    );
    event(
        catalog,
        AgentEvent::ActivityUpdated {
            activity: req.activity(),
            update: ActivityUpdate::TextSnapshot(
                Capture::Question {
                    interview,
                    revision: revision.into(),
                    question: questions()[index].clone(),
                    secret_batch: false,
                }
                .to_snapshot()
                .unwrap(),
            ),
        },
    );
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        // macOS의 임시 경로는 /var -> /private/var 링크를 거칠 수 있으므로,
        // descriptor 기반 저장소에는 fixture의 물리 경로를 전달한다.
        let root = fs::canonicalize(env::temp_dir())
            .expect("the interview fixture temp directory must resolve physically");
        let p = root.join(format!(
            "yo-interview-test-{}",
            working_copy::new_id().unwrap()
        ));
        fs::create_dir(&p).unwrap();
        fs::set_permissions(&p, Permissions::from_mode(0o700)).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// 표지 문자열만으로는 ModelWork가 질문 캡처나 복구 권한이 되지 않는다.
#[test]
fn genuine_request_is_required_and_noncanonical_profiles_are_rejected() {
    let (_, capture) = batch();
    let text = capture.to_snapshot().unwrap();
    let mut catalog = InterviewCatalog::default();
    event(
        &mut catalog,
        AgentEvent::ActivityStarted {
            activity: request(1).activity(),
            kind: ActivityKind::ModelWork,
        },
    );
    event(
        &mut catalog,
        AgentEvent::ActivityUpdated {
            activity: request(1).activity(),
            update: ActivityUpdate::TextSnapshot(text.clone()),
        },
    );
    assert!(catalog.interviews().is_empty());
    assert!(Capture::from_snapshot(&format!(" {text}")).is_err());
    assert!(
        Capture::from_snapshot(&text.replace("\"schema\":", "\"unknown\":0,\"schema\":")).is_err()
    );
    assert!(
        Capture::from_snapshot(&text.replace(
            "\"capture\":",
            "\"schema\":\"yo.interview-capture/v1\",\"capture\":"
        ))
        .is_err()
    );
    let mut secret = questions();
    secret[1].is_secret = true;
    assert!(Capture::batch(request(1), secret).is_err());
}

// 전체 답변 seal은 모든 최신 실제 응답과 값이 일치하고 최종 응답이 완료되어야 인정된다.
#[test]
fn final_seal_checks_every_answer_and_completion() {
    for corrupted in [false, true] {
        let (mut catalog, capture) = batch();
        let (a1, r1) = answered(&mut catalog, request(1), 0, "1", true);
        next_question(&mut catalog, &capture, request(2), 1);
        let (a2, r2) = answered(&mut catalog, request(2), 1, "두 번째", false);
        let (interview, revision) = capture.source();
        let mut answers = vec![a1, a2];
        if corrupted {
            answers[0].option_id = None;
            answers[0].text = "위조".into();
        }
        let seal = Capture::AcceptedAnswers {
            interview,
            revision: revision.into(),
            answers,
            answer_responses: vec![r1, r2.clone()],
            final_request: request(2),
            response_activity: r2.response_activity,
        };
        event(
            &mut catalog,
            AgentEvent::ActivityUpdated {
                activity: r2.response_activity,
                update: ActivityUpdate::TextSnapshot(seal.to_snapshot().unwrap()),
            },
        );
        assert!(catalog.interviews()[0].submitted.is_none());
        event(
            &mut catalog,
            AgentEvent::ActivityFinished {
                activity: r2.response_activity,
                outcome: ActivityOutcome::Completed,
            },
        );
        assert_eq!(catalog.interviews()[0].submitted.is_some(), !corrupted);
    }
}

// 이전 질문으로 이동하거나 더 오래된 답변을 재사용해도 최신 응답을 대신할 수 없다.
#[test]
fn navigation_and_old_answer_responses_cannot_seal() {
    let (mut catalog, capture) = batch();
    let (old, r1) = answered(&mut catalog, request(1), 0, "이전", true);
    next_question(&mut catalog, &capture, request(3), 0);
    let (_new, _rnew) = answered(&mut catalog, request(3), 0, "수정", true);
    next_question(&mut catalog, &capture, request(2), 1);
    let (a2, r2) = answered(&mut catalog, request(2), 1, "최종", false);
    let (interview, revision) = capture.source();
    let seal = Capture::AcceptedAnswers {
        interview,
        revision: revision.into(),
        answers: vec![old, a2],
        answer_responses: vec![r1, r2.clone()],
        final_request: request(2),
        response_activity: r2.response_activity,
    };
    event(
        &mut catalog,
        AgentEvent::ActivityUpdated {
            activity: r2.response_activity,
            update: ActivityUpdate::TextSnapshot(seal.to_snapshot().unwrap()),
        },
    );
    event(
        &mut catalog,
        AgentEvent::ActivityFinished {
            activity: r2.response_activity,
            outcome: ActivityOutcome::Completed,
        },
    );
    assert!(catalog.interviews()[0].submitted.is_none());
    let nav = ActivityResponse::PreviousQuestion {
        choice: None,
        draft: UserInput::new("미제출"),
    };
    assert!(questions()[0].project_response(&nav).is_err());
}

// CAS에서 패자의 편집 데이터와 이미 저장된 승자를 보존하고, 복구는 마지막 게시본을 읽는다.
#[test]
fn cas_conflict_preserves_winner_and_editable_loser() {
    let temp = Temp::new();
    let repo = InterviewRepository::open(&temp.0).unwrap();
    let (catalog, _) = batch();
    let mut copy = WorkingCopy::new(&catalog.interviews()[0]).unwrap();
    assert_eq!(repo.save(&copy, None, &catalog).unwrap(), 1);
    let mut loser = repo.load(&copy.copy_id).unwrap().unwrap();
    copy.answers[0].text = "승자".into();
    assert_eq!(repo.save(&copy, Some(1), &catalog).unwrap(), 2);
    loser.answers[0].text = "패자의 편집".into();
    assert!(matches!(
        repo.save(&loser, Some(1), &catalog),
        Err(InterviewError::Conflict)
    ));
    assert_eq!(loser.answers[0].text, "패자의 편집");
    assert_eq!(
        repo.load(&copy.copy_id).unwrap().unwrap().answers[0].text,
        "승자"
    );
    assert!(
        !fs::read_dir(&temp.0).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp"))
    );
}

// 새 초안 형식은 왕복 직렬화되지만 새 대화 전송과 재개는 허용하지 않는다.
#[test]
fn contextual_draft_is_canonical_and_cannot_be_sent_as_new_conversation() {
    let (catalog, _) = batch();
    let copy = WorkingCopy::new_contextual(&catalog.interviews()[0]).unwrap();
    assert!(copy.is_contextual_draft());
    let encoded = copy.encode().unwrap();
    assert!(encoded.starts_with(b"{\"schema\":\"yo.interview-draft/v1\""));
    assert_eq!(WorkingCopy::decode(&encoded).unwrap(), copy);
    assert!(copy.reopen().is_err());
    assert!(copy.new_conversation(&catalog).is_err());
    assert!(
        !WorkingCopy::new(&catalog.interviews()[0])
            .unwrap()
            .is_contextual_draft()
    );
}

// 종전 v4 초안은 디코딩만 유지하고 새 v1 초안으로 오인하거나 전송하지 않는다.
#[test]
fn old_contextual_draft_remains_readable_but_is_not_current() {
    let (catalog, _) = batch();
    let copy = WorkingCopy::new_contextual(&catalog.interviews()[0]).unwrap();
    let old_bytes = String::from_utf8(copy.encode().unwrap()).unwrap().replacen(
        "yo.interview-draft/v1",
        "yo.interview-working-copy/v4",
        1,
    );
    let old = WorkingCopy::decode(old_bytes.as_bytes()).unwrap();
    assert!(!old.is_contextual_draft());
    assert!(old.validate(&catalog).is_ok());
    assert!(old.reopen().is_err());
    assert!(old.new_conversation(&catalog).is_err());
}

// 종전 문맥 초안의 비밀 질문을 옛 복구 경로로 보내도 파일과 vault를 바꾸지 않는다.
#[test]
fn old_contextual_secret_draft_cannot_enter_legacy_recovery() {
    let temp = Temp::new();
    let copies = temp.0.join("copies");
    let vault = temp.0.join("vault");
    let key = temp.0.join("config").join("secret-recovery.key");
    let repository = InterviewRepository::open_with_recovery(&copies, vault, key.clone()).unwrap();
    let catalog = catalog_for(request(1), secret_questions());
    let copy = WorkingCopy::new_contextual(&catalog.interviews()[0]).unwrap();
    let bytes = String::from_utf8(copy.encode().unwrap()).unwrap().replacen(
        "yo.interview-draft/v1",
        "yo.interview-working-copy/v4",
        1,
    );
    let old = WorkingCopy::decode(bytes.as_bytes()).unwrap();
    repository.save(&old, None, &catalog).unwrap();
    let destination = SecretRecoveryDestination::managed(
        &crate::ProviderId::new("provider").unwrap(),
        &crate::ModelId::new("model").unwrap(),
        &crate::AccountId::new("account").unwrap(),
    );
    let result = repository.store_secret_recovery(
        &old,
        Some(old.generation),
        &catalog,
        "q2",
        &destination,
        &crate::SecretInput::new("must-not-be-stored").unwrap(),
    );
    assert!(
        matches!(result, Err(InterviewError::Invalid(message)) if message == "legacy secret recovery is unavailable for contextual drafts")
    );
    assert_eq!(repository.load(&old.copy_id).unwrap().unwrap(), old);
    assert!(!key.exists());
}

// 삭제는 저장된 최신 세대만 허용해 동시 편집 결과를 보존한다.
#[test]
fn contextual_draft_delete_requires_the_exact_persisted_generation() {
    let temp = Temp::new();
    let repo = InterviewRepository::open(&temp.0).unwrap();
    let (catalog, _) = batch();
    let mut copy = WorkingCopy::new_contextual(&catalog.interviews()[0]).unwrap();
    repo.save(&copy, None, &catalog).unwrap();
    copy.answers[0].text = "updated".into();
    repo.save(&copy, Some(1), &catalog).unwrap();
    assert!(matches!(repo.delete(&copy), Err(InterviewError::Conflict)));
    let latest = repo.load(&copy.copy_id).unwrap().unwrap();
    repo.delete(&latest).unwrap();
    assert!(repo.load(&copy.copy_id).unwrap().is_none());
}

// 새 초안 형식에 이전 비밀 복구 참조가 끼어들면 저장과 복원을 모두 거부한다.
#[test]
fn contextual_draft_rejects_legacy_secret_recovery_references() {
    let (catalog, _) = batch();
    let mut copy = WorkingCopy::new_contextual(&catalog.interviews()[0]).unwrap();
    copy.secret_recovery
        .push(recovery::SecretRecoveryReference::new(
            copy.current_question_id.clone(),
            crate::SubmissionId::new().unwrap().to_string(),
        ));
    assert!(copy.validate(&catalog).is_err());
    assert!(copy.encode().is_err());
    assert!(WorkingCopy::decode(&serde_json::to_vec(&copy).unwrap()).is_err());
}

// 심볼릭 링크, 느슨한 권한, 알 수 없는 형식의 데이터는 수정하거나 삭제하지 않고 거부한다.
#[test]
fn unsafe_storage_and_unknown_records_are_preserved() {
    let temp = Temp::new();
    let outside = Temp::new();
    let link = temp.0.join("link");
    symlink(&outside.0, &link).unwrap();
    assert!(InterviewRepository::open(&link).is_err());
    let repo = InterviewRepository::open(&temp.0).unwrap();
    let id = working_copy::new_id().unwrap();
    let path = temp.0.join(format!("{id}.json"));
    symlink(outside.0.join("value"), &path).unwrap();
    assert!(repo.load(&id).is_err());
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"{\"schema\":\"future\"}").unwrap();
    fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
    assert!(repo.load(&id).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"{\"schema\":\"future\"}");
    fs::set_permissions(&temp.0, Permissions::from_mode(0o755)).unwrap();
    assert!(InterviewRepository::open(&temp.0).is_err());
    assert!(repo.load(&id).is_err());
}

// 제출본 다시 열기는 새 UUID와 첫 generation을 만들고 원본의 제출 표시는 그대로 남긴다.
#[test]
fn reopen_retains_original_and_preview_limits_use_utf8_bytes() {
    let (catalog, _) = batch();
    let mut original = WorkingCopy::new(&catalog.interviews()[0]).unwrap();
    original.generation = 4;
    original.submission = Some(Submission::NewConversation {
        turn: request(1).activity().turn(),
        submission_id: working_copy::new_id().unwrap(),
        accepted_request_sequence: 1,
    });
    let reopened = original.reopen().unwrap();
    assert_ne!(original.copy_id, reopened.copy_id);
    assert_eq!(reopened.generation, 1);
    assert!(reopened.submission.is_none());
    assert!(original.submission.is_some());
    let mut copy = reopened;
    let overhead = copy.preview(&catalog).unwrap().len();
    copy.context = "가".repeat((PREVIEW_LIMIT - overhead - 20) / 3);
    assert!(copy.preview(&catalog).is_ok());
    copy.context.push_str(&"가".repeat(20));
    assert!(copy.preview(&catalog).is_err());
    copy.context = "x".repeat(COPY_LIMIT);
    assert!(copy.encode().is_err());
}

// 캡처된 원문 prompt가 질문과 선택지를 이미 포함하므로 복구 preview는 원문과
// 편집 답안을 각각 한 번만 보여 준다.
#[test]
fn preview_does_not_duplicate_the_captured_question() {
    let question = InterviewQuestion {
        id: "q1".into(),
        prompt: "제목\n\n어떤 답인가요?\n1. 선택 — 설명".into(),
        question: "어떤 답인가요?".into(),
        options: vec![InterviewOption {
            id: "1".into(),
            label: "선택".into(),
            description: "설명".into(),
        }],
        allow_free_text: true,
        allow_notes: true,
        is_secret: false,
    };
    let capture = Capture::batch(request(1), vec![question]).unwrap();
    let mut catalog = InterviewCatalog::default();
    event(
        &mut catalog,
        AgentEvent::ActivityStarted {
            activity: request(1).activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: request(1).request_id(),
            },
        },
    );
    event(
        &mut catalog,
        AgentEvent::ActivityUpdated {
            activity: request(1).activity(),
            update: ActivityUpdate::TextSnapshot(capture.to_snapshot().unwrap()),
        },
    );
    let mut copy = WorkingCopy::new(&catalog.interviews()[0]).unwrap();
    copy.answers[0].option_id = Some("1".into());

    assert_eq!(
        copy.preview(&catalog).unwrap(),
        "Interview questions and editable answers\n\n제목\n\n어떤 답인가요?\n1. 선택 — 설명\nAnswer: 선택\n\n"
    );
}

// 동시에 같은 generation을 쓰는 두 저장소 중 한 게시만 성공하며 파일은 완전한 정본이다.
#[test]
fn simultaneous_writers_publish_one_generation() {
    let temp = Temp::new();
    let (catalog, _) = batch();
    let copy = WorkingCopy::new(&catalog.interviews()[0]).unwrap();
    InterviewRepository::open(&temp.0)
        .unwrap()
        .save(&copy, None, &catalog)
        .unwrap();
    let ready = Arc::new(AtomicUsize::new(0));
    let repositories = [
        InterviewRepository::open(&temp.0).unwrap(),
        InterviewRepository::open(&temp.0).unwrap(),
    ];
    let results = thread::scope(|scope| {
        let handles = repositories
            .into_iter()
            .enumerate()
            .map(|(i, repo)| {
                let ready = ready.clone();
                let mut copy = copy.clone();
                let catalog = &catalog;
                scope.spawn(move || {
                    copy.context = i.to_string();
                    ready.fetch_add(1, Ordering::SeqCst);
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while ready.load(Ordering::SeqCst) != 2 {
                        assert!(Instant::now() < deadline, "writers did not become ready");
                        thread::sleep(Duration::from_millis(1));
                    }
                    repo.save(&copy, Some(1), catalog)
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let saved = InterviewRepository::open(&temp.0)
        .unwrap()
        .load(&copy.copy_id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.generation, 2);
    saved.validate(&catalog).unwrap();
}

// 첫 요청과 전체 batch의 마지막 segment까지 한 물리 게시에 들어가 재시작이 질문 일부를 추론하지
// 않는다.
#[test]
fn first_request_and_complete_capture_publish_in_one_physical_append() {
    use crate::{
        AgentRuntime, BackendEvent, BackendScriptStep, RuntimePoll, ScriptedBackend, SubmissionId,
        session_repository::{LocalSessionReader, LocalSessionRepository, read_stored_session},
    };
    let temp = Temp::new();
    let root = temp.0.join("sessions");
    let req = request(1);
    let session_id = req.activity().turn().session_id();
    let turn = req.activity().turn();
    let mut all = questions();
    all[1].question = "큰 질문".repeat(15000);
    let capture = Capture::batch(req, all).unwrap();
    let text = capture.to_snapshot().unwrap();
    assert!(text.len() > 64 * 1024);
    let start = AgentCommand::StartTurn {
        turn,
        input: UserInput::new("질문 요청"),
    };
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession { session_id }),
        BackendScriptStep::AcceptCommand(start.clone()),
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: req.activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: req.request_id(),
            },
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
            activity: req.activity(),
            update: ActivityUpdate::TextSnapshot(text),
        }),
    ]);
    let repository = LocalSessionRepository::open(&root, 16 * 1024 * 1024).unwrap();
    let journal = SessionJournal::with_repository_and_descriptor(
        Box::new(repository),
        crate::fixture_descriptor(session_id),
    );
    let mut runtime = AgentRuntime::with_journal(backend, journal);
    runtime.initialize_durability();
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    runtime
        .execute_submission(start, SubmissionId::new().unwrap())
        .unwrap();
    let before = fs::read_to_string(root.join(format!("{session_id}.jsonl")))
        .unwrap()
        .lines()
        .count();
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted { .. })
    ));
    let physical = fs::read_to_string(root.join(format!("{session_id}.jsonl"))).unwrap();
    assert_eq!(physical.lines().count(), before + 1);
    let reader = LocalSessionReader::open(&root).unwrap();
    let restored = read_stored_session(&reader, session_id).unwrap();
    assert_eq!(restored.interviews().interviews().len(), 1);
    assert_eq!(restored.interviews().interviews()[0].questions.len(), 2);
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityUpdated { .. })
    ));
}

// 제출 표시는 실제 최초 요청의 SubmissionId/operation과 durable 수락 sequence가 맞을 때만 생긴다.
#[test]
fn durable_first_turn_acceptance_is_correlated_and_recovers() {
    use crate::{
        AgentRuntime, BackendBindingEvidence, BackendCommandEvidence, BackendIdentity,
        BackendRequestEvidence, BackendScriptStep, ContinuationStrategy, ScriptedBackend,
        SubmissionId,
        session_repository::{LocalSessionReader, LocalSessionRepository, read_stored_session},
    };
    let temp = Temp::new();
    let root = temp.0.join("sessions");
    let turn = request(1).activity().turn();
    let session_id = turn.session_id();
    let binding = BackendBindingEvidence::new(
        "interview-fixture",
        "1",
        BackendIdentity::new("fixture.binding/v1", "binding"),
        BackendIdentity::new("fixture.model/v1", "model"),
        BackendIdentity::new("fixture.session/v1", "session"),
        ContinuationStrategy::BackendManagedState,
    );
    let evidence = BackendRequestEvidence::new(
        "fixture.request/v1",
        BackendIdentity::new("fixture.exchange/v1", "exchange"),
        BackendIdentity::new("fixture.accepted/v1", "accepted"),
    );
    let start = AgentCommand::StartTurn {
        turn,
        input: UserInput::new("명시적 preview"),
    };
    let id = SubmissionId::new().unwrap();
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::CreateSession { session_id },
            evidence: BackendCommandEvidence::BindingOpened(binding),
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: start.clone(),
            evidence: BackendCommandEvidence::RequestAccepted(evidence),
        },
    ]);
    let repository = LocalSessionRepository::open(&root, 16 * 1024 * 1024).unwrap();
    let journal = SessionJournal::with_repository_and_descriptor(
        Box::new(repository),
        crate::fixture_descriptor(session_id),
    );
    let mut runtime = AgentRuntime::with_journal(backend, journal);
    runtime.initialize_durability();
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    let reader = runtime.transcript_reader();
    assert!(reader.accepted_initial_submission(id).is_none());
    runtime.execute_submission(start, id).unwrap();
    let (accepted, seq) = reader
        .accepted_initial_submission(id)
        .expect("real durable first request");
    assert_eq!(accepted, turn);
    assert!(seq.get() > 0);
    assert!(
        reader
            .accepted_initial_submission(SubmissionId::new().unwrap())
            .is_none()
    );
    let stored =
        read_stored_session(&LocalSessionReader::open(&root).unwrap(), session_id).unwrap();
    assert_eq!(stored.accepted_initial_submission(id), Some((turn, seq)));
    let entries = runtime.journal().semantic_entries();
    let durable_cutoff = DurableCutoff::Known {
        journal_sequence: Some(seq),
        repository_sequence: RepositorySequence::new(1),
    };
    assert_eq!(
        initial_submission_evidence(
            &entries,
            id,
            crate::JournalDurability::Gap {
                durable_cutoff,
                cause: crate::DurabilityGapCause::Storage
            }
        ),
        Some((turn, seq))
    );
    assert!(
        initial_submission_evidence(
            &entries,
            id,
            crate::JournalDurability::Gap {
                durable_cutoff: DurableCutoff::Unknown,
                cause: crate::DurabilityGapCause::Storage
            }
        )
        .is_none()
    );
}

// UI가 아직 시작 이벤트를 읽지 않았어도 worker의 예약된 Turn은 새 대화 생성에서 busy다.
#[test]
fn reserved_worker_turn_is_busy_before_frontend_observation() {
    use crate::{
        AgentIntent, AgentSession, BackendScriptStep, CommandAdmission, InputSubmission,
        ScriptedBackend, SubmissionId,
    };
    let turn = request(1).activity().turn();
    let input = UserInput::new("예정된 입력");
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: turn.session_id(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn,
            input: input.clone(),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut session = AgentSession::start_for_test(backend, turn.session_id()).unwrap();
    let idle_deadline = Instant::now() + Duration::from_secs(2);
    while !session.is_idle_for_new_conversation() {
        assert!(Instant::now() < idle_deadline);
        thread::yield_now();
    }
    let admission = session
        .dispatch(AgentIntent::Submit(InputSubmission::new(
            SubmissionId::new().unwrap(),
            input,
        )))
        .unwrap();
    assert!(matches!(admission, CommandAdmission::Queued));
    assert!(!session.is_idle_for_new_conversation());
    let deadline = Instant::now() + Duration::from_secs(2);
    while session.take_submission_outcome().is_none() {
        assert!(Instant::now() < deadline);
        let _ = session.poll();
        thread::yield_now();
    }
    session.shutdown().unwrap();
}

// 전체 캡처의 정확한 UTF8 바이트 한도와 첫 초과를 확인하고 generation overflow는 게시본을 보존한다.
#[test]
fn complete_capture_limit_and_generation_overflow_preserve_boundaries() {
    let mut questions = questions();
    questions[0].question = "a".into();
    let base = Capture::batch(request(1), questions.clone())
        .unwrap()
        .to_snapshot()
        .unwrap()
        .len();
    let padding = CAPTURE_LIMIT - base + 1;
    questions[0].question = "가".repeat(padding / 3) + &"a".repeat(padding % 3);
    let exact = Capture::batch(request(1), questions.clone())
        .unwrap()
        .to_snapshot()
        .unwrap();
    assert_eq!(exact.len(), CAPTURE_LIMIT);
    assert!(Capture::from_snapshot(&exact).is_ok());
    questions[0].question.push('a');
    assert!(Capture::batch(request(1), questions).is_err());
    let temp = Temp::new();
    let (catalog, _) = batch();
    let mut copy = WorkingCopy::new(&catalog.interviews()[0]).unwrap();
    copy.generation = u64::MAX;
    let path = temp.0.join(format!("{}.json", copy.copy_id));
    let bytes = copy.encode().unwrap();
    fs::write(&path, &bytes).unwrap();
    fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
    assert!(
        InterviewRepository::open(&temp.0)
            .unwrap()
            .save(&copy, Some(u64::MAX), &catalog)
            .is_err()
    );
    assert_eq!(fs::read(path).unwrap(), bytes);
}

// 장애로 남은 우리 UUID temp만 exclusive lease에서 회수하고 정상 파일·unknown·symlink는 보존한다.
#[test]
fn abandoned_owned_attempts_are_reclaimed_without_purging_unknown_files() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let (catalog, _) = batch();
    let copy = WorkingCopy::new(&catalog.interviews()[0]).unwrap();
    let repo = InterviewRepository::open(&temp.0).unwrap();
    repo.save(&copy, None, &catalog).unwrap();
    let owned = temp.0.join(format!(
        ".{}.{}.tmp",
        copy.copy_id,
        working_copy::new_id().unwrap()
    ));
    fs::write(&owned, b"partial interrupted write").unwrap();
    fs::set_permissions(&owned, Permissions::from_mode(0o600)).unwrap();
    let unknown = temp.0.join("unknown.tmp");
    fs::write(&unknown, b"keep").unwrap();
    let unsafe_path = temp.0.join(format!(
        ".{}.{}.tmp",
        copy.copy_id,
        working_copy::new_id().unwrap()
    ));
    symlink(&unknown, &unsafe_path).unwrap();
    drop(repo);
    let reopened = InterviewRepository::open(&temp.0).unwrap();
    assert!(!owned.exists());
    assert_eq!(fs::read(&unknown).unwrap(), b"keep");
    assert!(
        fs::symlink_metadata(unsafe_path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(reopened.load(&copy.copy_id).unwrap().unwrap().generation, 1);
}

// genuine 완료 응답만 bounded 진단을 복구하며 미완료·중단·첫 초과 UTF8 byte는 권한이 없다.
#[test]
fn recovery_diagnostic_requires_completion_and_rejects_first_excess_byte() {
    let prefix = RECOVERY_UNAVAILABLE_RECEIPT_PREFIX;
    let capacity = RECOVERY_DIAGNOSTIC_LIMIT - prefix.len();
    let exact = format!(
        "{prefix}{}{}",
        "문".repeat(capacity / 3),
        "x".repeat(capacity % 3)
    );
    assert_eq!(exact.len(), RECOVERY_DIAGNOSTIC_LIMIT);
    for (line, completed, appended) in [
        (&exact, true, false),
        (&format!("{exact}x"), true, false),
        (&exact, false, false),
        (&exact, true, true),
    ] {
        let (mut catalog, _) = batch();
        let response = request(2).activity();
        catalog.observe_committed(&TranscriptRecord::CommandCommitted(
            AgentCommand::RespondToActivity {
                request: request(1),
                response: ActivityResponse::UserInput(UserInput::new("answer")),
            },
        ));
        event(
            &mut catalog,
            AgentEvent::ActivityStarted {
                activity: response,
                kind: ActivityKind::UserInputResponse {
                    request_id: request(1).request_id(),
                },
            },
        );
        event(
            &mut catalog,
            AgentEvent::ActivityUpdated {
                activity: response,
                update: ActivityUpdate::TextSnapshot(line.clone()),
            },
        );
        assert!(catalog.recovery_unavailable(request(1)).is_none());
        if appended {
            event(
                &mut catalog,
                AgentEvent::ActivityUpdated {
                    activity: response,
                    update: ActivityUpdate::TextDelta("x".into()),
                },
            );
        }
        event(
            &mut catalog,
            AgentEvent::ActivityFinished {
                activity: response,
                outcome: if completed {
                    ActivityOutcome::Completed
                } else {
                    ActivityOutcome::Interrupted
                },
            },
        );
        let actual = catalog.recovery_unavailable(request(1));
        assert_eq!(
            actual,
            (completed && !appended && line.len() == RECOVERY_DIAGNOSTIC_LIMIT)
                .then_some(line.as_str())
        );
        assert!(catalog.interviews()[0].submitted.is_none());
    }
}

// 비밀 질문이 포함된 캡처만 v2를 쓰고 공개 질문의 기존 v1 wire 형태는 유지하는지 확인한다.
#[test]
fn secret_capture_uses_v2_without_changing_later_question_wire_shape() {
    let public = questions();
    let public_capture = Capture::Question {
        interview: request(1),
        revision: Capture::batch(request(1), public.clone())
            .unwrap()
            .source()
            .1
            .to_owned(),
        question: public[0].clone(),
        secret_batch: false,
    };
    let public_text = public_capture.to_snapshot().unwrap();
    assert!(public_text.starts_with("{\"schema\":\"yo.interview-capture/v1\""));
    assert_eq!(
        public_text,
        r#"{"schema":"yo.interview-capture/v1","capture":{"kind":"question","interview":{"activity":{"turn":{"session_id":"01890f00-0000-7000-8000-000000000001","turn_id":1},"activity_id":1},"request_id":1},"revision":"sha256:1b719cdb5963bc3e5ff6c64785acd23df7c44e57854323aa45407dcbab45f852","question":{"id":"q1","prompt":"질문 1","question":"어떤 답인가요?","options":[{"id":"1","label":"선택","description":"설명"}],"allow_free_text":true,"allow_notes":true,"is_secret":false}}}"#
    );
    assert!(!public_text.contains("secret_batch"));
    assert_eq!(
        Capture::from_snapshot(&public_text)
            .unwrap()
            .to_snapshot()
            .unwrap(),
        public_text
    );

    let secret_capture = Capture::batch(request(1), secret_questions()).unwrap();
    let secret_text = secret_capture.to_snapshot().unwrap();
    assert!(secret_text.starts_with("{\"schema\":\"yo.interview-capture/v2\""));
    assert!(secret_text.contains("\"is_secret\":true"));
    assert!(!secret_text.contains("secret_batch"));

    let (interview, revision) = secret_capture.source();
    let public_later = Capture::Question {
        interview,
        revision: revision.to_owned(),
        question: secret_questions()[0].clone(),
        secret_batch: true,
    };
    let later_text = public_later.to_snapshot().unwrap();
    assert!(later_text.starts_with("{\"schema\":\"yo.interview-capture/v2\""));
    assert_eq!(
        later_text,
        r#"{"schema":"yo.interview-capture/v2","capture":{"kind":"question","interview":{"activity":{"turn":{"session_id":"01890f00-0000-7000-8000-000000000001","turn_id":1},"activity_id":1},"request_id":1},"revision":"sha256:22bc0e7e62147040e726afae96e8019a6a01887ebef823fc87fe51eb0e21336a","question":{"id":"q1","prompt":"질문 1","question":"어떤 답인가요?","options":[{"id":"1","label":"선택","description":"설명"}],"allow_free_text":true,"allow_notes":true,"is_secret":false}}}"#
    );
    assert!(!later_text.contains("secret_batch"));
    assert!(
        Capture::from_snapshot(&later_text)
            .unwrap()
            .is_secret_batch()
    );
}

// 비밀 답은 고정 마커로만 직렬화되고 복구 사본에서 새 대화를 시작할 수 없는지 확인한다.
#[test]
fn secret_answers_are_fixed_markers_and_working_copies_cannot_start_turns() {
    let questions = secret_questions();
    let capture = Capture::batch(request(1), questions.clone()).unwrap();
    let secret = &questions[1];
    let submitted = secret
        .project_response(&ActivityResponse::SecretInput(
            crate::SecretInput::new("value-that-must-not-be-serialized").unwrap(),
        ))
        .unwrap();
    assert_eq!(
        serde_json::to_string(&submitted).unwrap(),
        r#"{"question_id":"q2","secret":"submitted"}"#
    );
    assert_eq!(
        serde_json::to_string(&secret.empty_answer()).unwrap(),
        r#"{"question_id":"q2","secret":"reentry_required"}"#
    );
    let mut poisoned_marker = secret.empty_answer();
    poisoned_marker.text = "raw-secret-must-not-cross-the-wire".into();
    assert_eq!(
        serde_json::to_string(&poisoned_marker).unwrap(),
        r#"{"question_id":"q2","secret":"reentry_required"}"#
    );

    let mut catalog = InterviewCatalog::default();
    event(
        &mut catalog,
        AgentEvent::ActivityStarted {
            activity: request(1).activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: request(1).request_id(),
            },
        },
    );
    event(
        &mut catalog,
        AgentEvent::ActivityUpdated {
            activity: request(1).activity(),
            update: ActivityUpdate::TextSnapshot(capture.to_snapshot().unwrap()),
        },
    );
    let copy = WorkingCopy::new(&catalog.interviews()[0]).unwrap();
    let encoded = copy.encode().unwrap();
    assert!(
        String::from_utf8_lossy(&encoded).contains("\"schema\":\"yo.interview-working-copy/v2\"")
    );
    assert!(!String::from_utf8_lossy(&encoded).contains("value-that-must-not-be-serialized"));
    assert!(copy.new_conversation(&catalog).is_err());
    assert!(
        copy.preview(&catalog)
            .unwrap()
            .contains("[secret re-entry required]")
    );

    let submission_id = working_copy::new_id().unwrap();
    let turn = request(1).activity().turn();
    let forbidden_submission = format!(
        r#"{{"kind":"new_conversation","turn":{{"session_id":"{}","turn_id":1}},"submission_id":"{}","accepted_request_sequence":1}}"#,
        turn.session_id(),
        submission_id
    );
    let mutated = String::from_utf8(encoded).unwrap().replace(
        "\"submission\":null",
        &format!("\"submission\":{forbidden_submission}"),
    );
    assert!(WorkingCopy::decode(mutated.as_bytes()).is_err());
}

// opt-in 저장은 working-copy에 opaque 참조만 남기고, 고정 길이 암호문을 먼저 게시한
// 다음 CAS generation을 올린다. 다른 live request라도 공개 batch와 destination이 같을 때만
// 값을 hidden input으로 복구한다.
#[test]
fn opted_in_secret_recovery_is_padded_bound_and_explicitly_forgotten() {
    let temp = Temp::new();
    let copies = temp.0.join("copies");
    let vault = temp.0.join("vault");
    let key = temp.0.join("config").join("secret-recovery.key");
    let repository =
        InterviewRepository::open_with_recovery(&copies, vault.clone(), key.clone()).unwrap();
    let source = catalog_for(request(1), secret_questions());
    let live = catalog_for(request(20), secret_questions());
    let mut copy = WorkingCopy::new(&source.interviews()[0]).unwrap();
    copy.generation = repository.save(&copy, None, &source).unwrap();
    let destination = SecretRecoveryDestination::managed(
        &crate::ProviderId::new("provider").unwrap(),
        &crate::ModelId::new("model").unwrap(),
        &crate::AccountId::new("authenticated-account").unwrap(),
    );
    let secret_text = "복구할-비밀";
    copy = repository
        .store_secret_recovery(
            &copy,
            Some(copy.generation),
            &source,
            "q2",
            &destination,
            &crate::SecretInput::new(secret_text).unwrap(),
        )
        .unwrap();

    let encoded = String::from_utf8(copy.encode().unwrap()).unwrap();
    assert!(encoded.contains("\"schema\":\"yo.interview-working-copy/v3\""));
    assert!(encoded.contains("\"state\":\"recovery_available\""));
    for forbidden in [secret_text, "ciphertext", "nonce", "authenticated-account"] {
        assert!(
            !encoded.contains(forbidden),
            "{forbidden} leaked into {encoded}"
        );
    }
    let entry = fs::read_dir(&vault)
        .unwrap()
        .find_map(|entry| {
            let entry = entry.unwrap();
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".entry")
                .then_some(entry.path())
        })
        .unwrap();
    assert_eq!(
        fs::metadata(&entry).unwrap().len(),
        recovery::ENTRY_BYTES as u64
    );
    assert_eq!(
        fs::metadata(&key).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(fs::read(&key).unwrap().len(), 32);
    assert!(
        !fs::read(&entry)
            .unwrap()
            .windows(secret_text.len())
            .any(|window| window == secret_text.as_bytes())
    );

    assert!(
        repository
            .recovery_available(&copy, &source, &live.interviews()[0], "q2", &destination,)
            .unwrap()
    );
    let recovered = repository
        .recover_secret(&copy, &source, &live.interviews()[0], "q2", &destination)
        .unwrap();
    assert_eq!(recovered.expose(), secret_text);
    let mismatched = SecretRecoveryDestination::managed(
        &crate::ProviderId::new("provider").unwrap(),
        &crate::ModelId::new("other-model").unwrap(),
        &crate::AccountId::new("authenticated-account").unwrap(),
    );
    assert!(
        repository
            .recovery_available(&copy, &source, &live.interviews()[0], "q2", &mismatched,)
            .is_err()
    );
    assert!(
        repository
            .recover_secret(&copy, &source, &live.interviews()[0], "q2", &mismatched,)
            .is_err()
    );

    let mut damaged = fs::read(&entry).unwrap();
    damaged[recovery::ENTRY_BYTES / 2] ^= 0x40;
    fs::write(&entry, damaged).unwrap();
    assert!(
        repository
            .recovery_available(&copy, &source, &live.interviews()[0], "q2", &destination,)
            .is_err()
    );

    let update = repository
        .forget_secret_recovery(&copy, Some(copy.generation), &source, "q2")
        .unwrap();
    assert!(update.cleanup_warning.is_none());
    assert!(!entry.exists());
    let forgotten = String::from_utf8(update.copy.encode().unwrap()).unwrap();
    assert!(forgotten.contains("\"schema\":\"yo.interview-working-copy/v2\""));
    assert!(!forgotten.contains("secret_recovery"));
}

// key 손실 상태에서 살아 있는 ciphertext 위로 새 key를 만들면 기존 참조가 조용히
// 다른 비밀로 바뀔 수 있으므로 저장을 닫고 공개 v3 사본을 그대로 둔다.
#[test]
fn missing_recovery_key_is_not_regenerated_over_surviving_ciphertext() {
    let temp = Temp::new();
    let copies = temp.0.join("copies");
    let vault = temp.0.join("vault");
    let key = temp.0.join("config").join("secret-recovery.key");
    let repository =
        InterviewRepository::open_with_recovery(&copies, vault.clone(), key.clone()).unwrap();
    let source = catalog_for(request(1), secret_questions());
    let mut copy = WorkingCopy::new(&source.interviews()[0]).unwrap();
    copy.generation = repository.save(&copy, None, &source).unwrap();
    let destination = SecretRecoveryDestination::managed(
        &crate::ProviderId::new("provider").unwrap(),
        &crate::ModelId::new("model").unwrap(),
        &crate::AccountId::new("authenticated-account").unwrap(),
    );
    copy = repository
        .store_secret_recovery(
            &copy,
            Some(copy.generation),
            &source,
            "q2",
            &destination,
            &crate::SecretInput::new("first").unwrap(),
        )
        .unwrap();
    let entry = fs::read_dir(&vault)
        .unwrap()
        .find_map(|entry| {
            let entry = entry.unwrap();
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".entry")
                .then_some(entry.path())
        })
        .unwrap();
    fs::rename(entry, vault.join("unclassified-survivor")).unwrap();
    fs::remove_file(&key).unwrap();

    let error = repository
        .store_secret_recovery(
            &copy,
            Some(copy.generation),
            &source,
            "q2",
            &destination,
            &crate::SecretInput::new("second").unwrap(),
        )
        .unwrap_err();

    assert!(error.to_string().contains("key is missing"));
    assert!(!key.exists());
    assert_eq!(
        repository
            .load(&copy.copy_id)
            .unwrap()
            .unwrap()
            .encode()
            .unwrap(),
        copy.encode().unwrap()
    );
}

// Expiry is repository-wide and does not wait for the user to reopen the copy.
// The public reference is published away before its encrypted entry is removed.
#[test]
fn maintenance_expires_unopened_secret_recovery() {
    let temp = Temp::new();
    let copies = temp.0.join("copies");
    let vault = temp.0.join("vault");
    let key = temp.0.join("config").join("secret-recovery.key");
    let repository = InterviewRepository::open_with_recovery(&copies, vault.clone(), key).unwrap();
    let source = catalog_for(request(1), secret_questions());
    let mut copy = WorkingCopy::new(&source.interviews()[0]).unwrap();
    copy.generation = repository.save(&copy, None, &source).unwrap();
    let destination = SecretRecoveryDestination::managed(
        &crate::ProviderId::new("provider").unwrap(),
        &crate::ModelId::new("model").unwrap(),
        &crate::AccountId::new("authenticated-account").unwrap(),
    );
    copy = repository
        .store_secret_recovery(
            &copy,
            Some(copy.generation),
            &source,
            "q2",
            &destination,
            &crate::SecretInput::new("expires-without-open").unwrap(),
        )
        .unwrap();
    let entry = fs::read_dir(&vault)
        .unwrap()
        .find_map(|entry| {
            let entry = entry.unwrap();
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".entry")
                .then_some(entry.path())
        })
        .unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&entry)
        .unwrap()
        .set_times(
            fs::FileTimes::new().set_modified(
                SystemTime::now()
                    .checked_sub(recovery::EXPIRY + Duration::from_secs(1))
                    .unwrap(),
            ),
        )
        .unwrap();

    let maintenance = repository.maintain_secret_recovery().unwrap();
    assert_eq!(maintenance.expired_references, 1);
    assert!(maintenance.warning.is_none());
    assert!(!entry.exists());
    let current = repository.load(&copy.copy_id).unwrap().unwrap();
    assert!(!current.has_any_secret_recovery());
    assert!(current.generation > copy.generation);
}

// 혼합 질문의 비밀 답이 값 없는 제출 영수증을 받은 뒤에만 완료 처리되는지 확인한다.
#[test]
fn catalog_seals_mixed_secret_answers_only_from_the_payload_free_receipt() {
    let questions = secret_questions();
    let capture = Capture::batch(request(1), questions.clone()).unwrap();
    let mut catalog = InterviewCatalog::default();
    event(
        &mut catalog,
        AgentEvent::ActivityStarted {
            activity: request(1).activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: request(1).request_id(),
            },
        },
    );
    event(
        &mut catalog,
        AgentEvent::ActivityUpdated {
            activity: request(1).activity(),
            update: ActivityUpdate::TextSnapshot(capture.to_snapshot().unwrap()),
        },
    );
    let (public_answer, public_response) = answered(&mut catalog, request(1), 0, "public", true);
    let (interview, revision) = capture.source();
    let secret_request = request(2);
    event(
        &mut catalog,
        AgentEvent::ActivityStarted {
            activity: secret_request.activity(),
            kind: ActivityKind::UserInputRequest {
                request_id: secret_request.request_id(),
            },
        },
    );
    event(
        &mut catalog,
        AgentEvent::ActivityUpdated {
            activity: secret_request.activity(),
            update: ActivityUpdate::TextSnapshot(
                Capture::Question {
                    interview,
                    revision: revision.into(),
                    question: questions[1].clone(),
                    secret_batch: true,
                }
                .to_snapshot()
                .unwrap(),
            ),
        },
    );
    let secret_response_activity = request(12).activity();
    catalog.observe_committed(&TranscriptRecord::CommandCommitted(
        AgentCommand::RespondToActivity {
            request: secret_request,
            response: ActivityResponse::SecretInputSubmitted,
        },
    ));
    event(
        &mut catalog,
        AgentEvent::ActivityStarted {
            activity: secret_response_activity,
            kind: ActivityKind::UserInputResponse {
                request_id: secret_request.request_id(),
            },
        },
    );
    let secret_answer = questions[1]
        .project_response(&ActivityResponse::SecretInputSubmitted)
        .unwrap();
    let secret_response = AnswerResponse {
        question_id: "q2".into(),
        request: secret_request,
        response_activity: secret_response_activity,
    };
    event(
        &mut catalog,
        AgentEvent::ActivityUpdated {
            activity: secret_response_activity,
            update: ActivityUpdate::TextSnapshot(
                Capture::AcceptedAnswers {
                    interview,
                    revision: revision.into(),
                    answers: vec![public_answer, secret_answer],
                    answer_responses: vec![public_response, secret_response],
                    final_request: secret_request,
                    response_activity: secret_response_activity,
                }
                .to_snapshot()
                .unwrap(),
            ),
        },
    );
    assert_eq!(
        catalog.answer_receipt(secret_response_activity).as_deref(),
        Some("질문 1\nAnswer: public\n질문 2\nAnswer: [secret submitted]\n")
    );
    event(
        &mut catalog,
        AgentEvent::ActivityFinished {
            activity: secret_response_activity,
            outcome: ActivityOutcome::Completed,
        },
    );
    assert_eq!(
        catalog.interviews()[0].submitted,
        Some((secret_request, secret_response_activity))
    );
}
