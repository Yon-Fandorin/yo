use std::{
    num::NonZeroU64,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
};

use super::*;
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
        let p = std::env::temp_dir().join(format!(
            "yo-interview-test-{}",
            working_copy::new_id().unwrap()
        ));
        std::fs::create_dir(&p).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
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
        !std::fs::read_dir(&temp.0).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp"))
    );
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
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"{\"schema\":\"future\"}").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(repo.load(&id).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"{\"schema\":\"future\"}");
    std::fs::set_permissions(&temp.0, std::fs::Permissions::from_mode(0o755)).unwrap();
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
    let ready = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let repositories = [
        InterviewRepository::open(&temp.0).unwrap(),
        InterviewRepository::open(&temp.0).unwrap(),
    ];
    let results = std::thread::scope(|scope| {
        let handles = repositories
            .into_iter()
            .enumerate()
            .map(|(i, repo)| {
                let ready = ready.clone();
                let mut copy = copy.clone();
                let catalog = &catalog;
                scope.spawn(move || {
                    copy.context = i.to_string();
                    ready.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                    while ready.load(std::sync::atomic::Ordering::SeqCst) != 2 {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "writers did not become ready"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(1));
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
    let journal = crate::journal::SessionJournal::with_repository_and_descriptor(
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
    let before = std::fs::read_to_string(root.join(format!("{session_id}.jsonl")))
        .unwrap()
        .lines()
        .count();
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted { .. })
    ));
    let physical = std::fs::read_to_string(root.join(format!("{session_id}.jsonl"))).unwrap();
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
    let journal = crate::journal::SessionJournal::with_repository_and_descriptor(
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
    let durable_cutoff = crate::session_repository::DurableCutoff::Known {
        journal_sequence: Some(seq),
        repository_sequence: crate::session_repository::RepositorySequence::new(1),
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
                durable_cutoff: crate::session_repository::DurableCutoff::Unknown,
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
    assert!(session.is_idle_for_new_conversation());
    let admission = session
        .dispatch(AgentIntent::Submit(InputSubmission::new(
            SubmissionId::new().unwrap(),
            input,
        )))
        .unwrap();
    assert!(matches!(admission, CommandAdmission::Queued));
    assert!(!session.is_idle_for_new_conversation());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while session.take_submission_outcome().is_none() {
        assert!(std::time::Instant::now() < deadline);
        let _ = session.poll();
        std::thread::yield_now();
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
    std::fs::write(&path, &bytes).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        InterviewRepository::open(&temp.0)
            .unwrap()
            .save(&copy, Some(u64::MAX), &catalog)
            .is_err()
    );
    assert_eq!(std::fs::read(path).unwrap(), bytes);
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
    std::fs::write(&owned, b"partial interrupted write").unwrap();
    std::fs::set_permissions(&owned, std::fs::Permissions::from_mode(0o600)).unwrap();
    let unknown = temp.0.join("unknown.tmp");
    std::fs::write(&unknown, b"keep").unwrap();
    let unsafe_path = temp.0.join(format!(
        ".{}.{}.tmp",
        copy.copy_id,
        working_copy::new_id().unwrap()
    ));
    symlink(&unknown, &unsafe_path).unwrap();
    drop(repo);
    let reopened = InterviewRepository::open(&temp.0).unwrap();
    assert!(!owned.exists());
    assert_eq!(std::fs::read(&unknown).unwrap(), b"keep");
    assert!(
        std::fs::symlink_metadata(unsafe_path)
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
