use std::{
    env,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use yo_backend::BackendAdapter as AgentBackend;
use yo_core::{
    ActivityKind, ActivityQuestion, ActivityRequestRef, AgentCommand, BackendEvent, BackendPoll,
    ModelConnectorEvent, ModelConnectorInputItem, ModelConnectorInputRole, ModelConnectorRequest,
    ToolApprovalRequirement, ToolDefinition, ToolSemanticAdmission, ToolSemanticAdmissionError,
    UserInput,
    session_repository::{
        AppendError, AppendReceipt, DurableRecord, DurableRecordKind, LocalSessionRepository,
        RepositoryEntry, RepositoryError, RepositorySequence, SessionRepository,
        SessionWriterRepository, recover_stored_session_continuation,
    },
};

use super::{
    super::support::{
        FixedTokenCounter, MockConnector, MockHost, binding, context_profile, event_rounds,
        registry, turn,
    },
    fixtures::{completed_text_round, tool_call_round, turn_number},
};
use crate::backend::{NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices};

struct SelectiveAdmission;

impl ToolSemanticAdmission for SelectiveAdmission {
    fn admit_arguments(
        &self,
        _: &ToolDefinition,
        arguments: &str,
    ) -> Result<String, ToolSemanticAdmissionError> {
        if arguments.contains("rejected-canary") {
            Err(ToolSemanticAdmissionError::new("private diagnostic canary"))
        } else if arguments.contains("invalid-admitted") {
            Ok("{".to_owned())
        } else {
            Ok(arguments.to_owned())
        }
    }

    fn admit_output(
        &self,
        _: &ToolDefinition,
        output: &str,
    ) -> Result<String, ToolSemanticAdmissionError> {
        Ok(output.to_owned())
    }
}

fn rejected_round() -> Vec<ModelConnectorEvent> {
    vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: "failed".into(),
        },
        ModelConnectorEvent::TextDelta {
            output_index: 0,
            item_id: "partial".into(),
            content_index: 0,
            delta: "partial-response-canary".into(),
        },
        ModelConnectorEvent::FunctionCallStarted {
            output_index: 1,
            item_id: "sibling".into(),
            call_id: "undispatched-sibling".into(),
            name: "read_file".into(),
        },
        ModelConnectorEvent::FunctionCallDone {
            output_index: 1,
            item_id: "sibling".into(),
            call_id: "undispatched-sibling".into(),
            name: "read_file".into(),
            arguments: r#"{"path":"undispatched-sibling-canary"}"#.into(),
        },
        ModelConnectorEvent::FunctionCallStarted {
            output_index: 2,
            item_id: "rejected".into(),
            call_id: "rejected-call".into(),
            name: "read_file".into(),
        },
        ModelConnectorEvent::FunctionCallDone {
            output_index: 2,
            item_id: "rejected".into(),
            call_id: "rejected-call".into(),
            name: "read_file".into(),
            arguments: r#"{"path":"rejected-canary"}"#.into(),
        },
    ]
}

fn backend(
    rounds: Vec<Vec<ModelConnectorEvent>>,
    requests: Arc<Mutex<Vec<ModelConnectorRequest>>>,
    starts: Arc<Mutex<usize>>,
) -> NativeModelBackend {
    NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(rounds),
            requests,
        }),
        binding(),
        registry(ToolApprovalRequirement::Automatic),
        NativeModelBackendServices::new(
            Box::new(yo_core::admit_standard_complete_binding),
            Some(Box::new(SelectiveAdmission)),
            Box::new(MockHost::with_start_counter(starts)),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig {
            ask_user_enabled: true,
            ..NativeModelBackendConfig::default()
        },
    )
    .unwrap()
}

fn next_event(backend: &mut NativeModelBackend) -> BackendEvent {
    for _ in 0..256 {
        if let BackendPoll::Event(event) = backend.poll_event().unwrap() {
            return event;
        }
    }
    panic!("bounded backend event wait exhausted")
}

fn preserved_input() -> Vec<ModelConnectorInputItem> {
    vec![
        ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: "original input".into(),
            refusal: None,
        },
        ModelConnectorInputItem::FunctionCall {
            call_id: "call-1".into(),
            name: "read_file".into(),
            arguments: r#"{"path":"README.md"}"#.into(),
        },
        ModelConnectorInputItem::FunctionCallOutput {
            call_id: "call-1".into(),
            output: r#"{"contents":"ok"}"#.into(),
        },
    ]
}

const QUESTION_ARGUMENTS: &str = r#"{"title":"Choose","question":"Which path?","choices":[{"label":"First","description":"One"}]}"#;

fn ordinary_question_round() -> Vec<ModelConnectorEvent> {
    vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: "question".into(),
        },
        ModelConnectorEvent::FunctionCallStarted {
            output_index: 0,
            item_id: "question-item".into(),
            call_id: "question-call".into(),
            name: "ask_user".into(),
        },
        ModelConnectorEvent::FunctionCallDone {
            output_index: 0,
            item_id: "question-item".into(),
            call_id: "question-call".into(),
            name: "ask_user".into(),
            arguments: QUESTION_ARGUMENTS.into(),
        },
        super::super::support::completed("question"),
    ]
}

fn preserved_question_input(source: &str) -> Vec<ModelConnectorInputItem> {
    let output = match source {
        "answered" => serde_json::json!({"schema":"yo.ask-user-result/v1","status":"answered","kind":"text","text":"answer-canary"}),
        "choice" => serde_json::json!({"schema":"yo.ask-user-result/v1","status":"answered","kind":"choice","choice":1,"label":"First","notes":"note-canary"}),
        _ => serde_json::json!({"schema":"yo.ask-user-result/v1","status":"unanswered"}),
    }.to_string();
    vec![
        ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: "original input".into(),
            refusal: None,
        },
        ModelConnectorInputItem::FunctionCall {
            call_id: "question-call".into(),
            name: "ask_user".into(),
            arguments: QUESTION_ARGUMENTS.into(),
        },
        ModelConnectorInputItem::FunctionCallOutput {
            call_id: "question-call".into(),
            output,
        },
    ]
}

struct QuestionMutationRepository {
    inner: LocalSessionRepository,
    mutation: &'static str,
    request_activity: u64,
    response_activity: u64,
    changed: AtomicBool,
}
impl SessionRepository for QuestionMutationRepository {
    fn append(
        &mut self,
        session: yo_core::SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        self.inner.append(session, record)
    }
    fn read_after(
        &self,
        session: yo_core::SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        self.inner
            .read_after(session, sequence, limit)
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|entry| {
                        let mut value: serde_json::Value =
                            serde_json::from_str(entry.record().payload()).unwrap();
                        if mutate_question_evidence(
                            &mut value,
                            self.mutation,
                            self.request_activity,
                            self.response_activity,
                        ) {
                            self.changed.store(true, Ordering::SeqCst);
                        }
                        let record = match entry.record().kind() {
                            DurableRecordKind::Incremental => {
                                DurableRecord::incremental(value.to_string())
                            },
                            DurableRecordKind::Snapshot => {
                                DurableRecord::snapshot(value.to_string())
                            },
                        };
                        RepositoryEntry::new(entry.sequence(), record)
                    })
                    .collect()
            })
    }
}
impl SessionWriterRepository for QuestionMutationRepository {
    fn acquire_session_writer(
        &mut self,
        session: yo_core::SessionId,
    ) -> Result<(), RepositoryError> {
        self.inner.acquire_session_writer(session)
    }
}

fn mutate_question_evidence(
    value: &mut serde_json::Value,
    mutation: &str,
    request_activity: u64,
    response_activity: u64,
) -> bool {
    let before = value.clone();
    if let serde_json::Value::Object(object) = value {
        if matches!(mutation, "secret" | "malformed-question")
            && let Some(snapshot) = object.get("text").and_then(serde_json::Value::as_str)
            && let Some(mut question) = ActivityQuestion::from_snapshot(snapshot)
        {
            let bytes = snapshot.len();
            let changed = if mutation == "secret" {
                question.is_secret = true;
                question.allow_unanswered = false;
                question.allow_notes = false;
                question.choices.clear();
                let mut snapshot = question.to_snapshot().unwrap();
                assert!(snapshot.len() <= bytes);
                snapshot.push_str(&" ".repeat(bytes - snapshot.len()));
                assert!(
                    ActivityQuestion::from_snapshot(&snapshot)
                        .unwrap()
                        .is_secret
                );
                snapshot
            } else {
                " ".repeat(bytes)
            };
            assert_eq!(
                changed.len(),
                bytes,
                "keep valid durable segment byte accounting"
            );
            object.insert("text".into(), changed.into());
        }
        let kind = object
            .get("type")
            .and_then(serde_json::Value::as_str)
            .or_else(|| object.get("kind").and_then(serde_json::Value::as_str))
            .map(str::to_owned);
        match (kind.as_deref(), mutation) {
            (Some("respond_to_activity"), "wrong-request") => {
                object.get_mut("request").unwrap()["request_id"] = 999.into();
            },
            (Some("user_input_response"), "wrong-response") => {
                object.insert("request_id".into(), 999.into());
            },
            (Some("activity_finished"), "incomplete-request" | "incomplete-response") => {
                let id = if mutation == "incomplete-request" {
                    request_activity
                } else {
                    response_activity
                };
                if object["activity"]["activity_id"] == id {
                    object.insert("outcome".into(), serde_json::json!({"type":"failed","code":null,"message":"incomplete question"}));
                }
            },
            (Some("function_call_output"), "malformed-result" | "mismatched-result")
                if object.get("call_id").and_then(serde_json::Value::as_str)
                    == Some("question-call") =>
            {
                object.insert("output".into(), if mutation == "malformed-result" { "not json".into() } else { serde_json::json!({"schema":"yo.ask-user-result/v1","status":"answered","kind":"text","text":"forged"}).to_string().into() });
            },
            _ => {},
        }
        for child in object.values_mut() {
            mutate_question_evidence(child, mutation, request_activity, response_activity);
        }
    } else if let serde_json::Value::Array(items) = value {
        for item in items {
            mutate_question_evidence(item, mutation, request_activity, response_activity);
        }
    }
    *value != before
}

fn assert_question_settlement_mutations_fail_closed(storage: &Path, session: yo_core::SessionId) {
    let disk = LocalSessionRepository::open(storage, 16 * 1024 * 1024).unwrap();
    let wire = disk
        .read_after(session, None, 256)
        .unwrap()
        .into_iter()
        .map(|entry| serde_json::from_str::<serde_json::Value>(entry.record().payload()).unwrap())
        .collect::<Vec<_>>();
    let activity_id = |kind: &str| {
        wire.iter()
            .flat_map(|commit| commit["records"].as_array().unwrap())
            .find_map(|record| {
                let event = &record["event"];
                (event["type"] == "activity_started" && event["kind"]["type"] == kind)
                    .then(|| event["activity"]["activity_id"].as_u64().unwrap())
            })
            .unwrap()
    };
    let request_activity = activity_id("user_input_request");
    let response_activity = activity_id("user_input_response");
    drop(disk);
    for mutation in [
        "wrong-request",
        "wrong-response",
        "secret",
        "malformed-question",
        "incomplete-request",
        "incomplete-response",
        "malformed-result",
        "mismatched-result",
    ] {
        let mut repository = QuestionMutationRepository {
            inner: LocalSessionRepository::open(storage, 16 * 1024 * 1024).unwrap(),
            mutation,
            request_activity,
            response_activity,
            changed: AtomicBool::new(false),
        };
        assert!(
            recover_stored_session_continuation(&mut repository, session).is_err(),
            "accepted {mutation}"
        );
        assert!(
            repository.changed.load(Ordering::SeqCst),
            "mutation {mutation} was not exercised"
        );
    }
}

// 앞서 닫힌 tool 묶음과 확정된 steer를 실패 후보에 담고, Core 반환 경계 이후
// 추가 poll 없이 StartTurn이 먼저 와도 다음 실제 request에 정확히 한 번 포함합니다.
#[test]
fn local_failure_next_request_preserves_closed_context_and_committed_steer() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let starts = Arc::new(Mutex::new(0));
    let mut backend = backend(
        vec![
            tool_call_round(),
            rejected_round(),
            completed_text_round("next", "done"),
        ],
        requests.clone(),
        starts.clone(),
    );
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("original input"),
        })
        .unwrap();
    loop {
        if matches!(
            next_event(&mut backend),
            BackendEvent::ContextActiveSuffixCompleted { .. }
        ) {
            break;
        }
    }
    backend
        .execute_command(AgentCommand::SteerTurn {
            turn: turn(),
            input: UserInput::from("accepted correction"),
        })
        .unwrap();
    backend.commit_prepared_command().unwrap();
    let proposal = loop {
        let event = next_event(&mut backend);
        if matches!(event, BackendEvent::LocalArgumentRejectionPrepared { .. }) {
            break event;
        }
    };
    let BackendEvent::LocalArgumentRejectionPrepared {
        replay: Some(delta),
        ..
    } = proposal
    else {
        panic!("nonempty closed source must carry a delta")
    };
    assert_eq!(delta.items().len(), 4);
    assert_eq!(
        delta.items().last(),
        Some(&UserInput::from("accepted correction").model_replay_item())
    );
    assert!(
        backend.replay.items().is_empty(),
        "candidate promoted before serialized Core return"
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "failure dispatched an automatic request"
    );
    assert_eq!(
        *starts.lock().unwrap(),
        1,
        "rejected call reached execution"
    );
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn_number(2),
            input: UserInput::from("next input"),
        })
        .unwrap();
    let mut expected = preserved_input();
    expected.extend(
        ["accepted correction", "next input"]
            .into_iter()
            .map(|content| ModelConnectorInputItem::Message {
                role: ModelConnectorInputRole::User,
                content: content.into(),
                refusal: None,
            }),
    );
    assert_eq!(&requests.lock().unwrap()[2].input()[1..], expected);
    assert_eq!(*starts.lock().unwrap(), 1);
    backend.shutdown().unwrap();
}

// 첫 input만 있거나 실패 요청 이후에 새 steer가 commit되면 이전 source로 잘라서
// 정산하지 않습니다. 전송·JSON 구조 오류도 typed argument 거절과 구분합니다.
#[test]
fn local_failure_without_eligible_source_remains_unsettled() {
    for case in [
        "input-only",
        "late-steer",
        "malformed-json",
        "identity",
        "invalid-admitted",
        "provider-failure",
    ] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let starts = Arc::new(Mutex::new(0));
        let mut rejected = rejected_round();
        if case == "malformed-json" {
            let ModelConnectorEvent::FunctionCallDone { arguments, .. } =
                rejected.last_mut().unwrap()
            else {
                unreachable!()
            };
            *arguments = "{".into();
        }
        if matches!(case, "identity" | "invalid-admitted") {
            let ModelConnectorEvent::FunctionCallDone {
                name, arguments, ..
            } = rejected.last_mut().unwrap()
            else {
                unreachable!()
            };
            if case == "identity" {
                *name = "changed-tool-name".into();
            } else {
                *arguments = r#"{"path":"invalid-admitted"}"#.into();
            }
        }
        if case == "provider-failure" {
            rejected = vec![
                ModelConnectorEvent::ResponseCreated {
                    response_id: "failed".into(),
                },
                ModelConnectorEvent::Terminal {
                    response_id: "failed".into(),
                    status: yo_core::ModelConnectorTerminal::Failed {
                        code: Some("provider-failure".into()),
                        request_failure: yo_core::ModelRequestFailureKind::Protocol,
                    },
                    usage: yo_core::ResponsesUsage {
                        input_tokens: None,
                        output_tokens: None,
                        total_tokens: None,
                        reasoning_tokens: None,
                        cache_read_input_tokens: yo_core::CacheReadInputTokens::Unsupported,
                    },
                },
            ];
        }
        let rounds = if case == "input-only" {
            vec![rejected]
        } else {
            vec![tool_call_round(), rejected]
        };
        let mut backend = backend(rounds, requests.clone(), starts);
        backend
            .execute_command(AgentCommand::CreateSession {
                session_id: turn().session_id(),
            })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("original input"),
            })
            .unwrap();
        loop {
            let event = next_event(&mut backend);
            if case == "late-steer" && matches!(event, BackendEvent::ModelRequestAccepted { .. }) {
                backend
                    .execute_command(AgentCommand::SteerTurn {
                        turn: turn(),
                        input: UserInput::from("late correction"),
                    })
                    .unwrap();
                backend.commit_prepared_command().unwrap();
            }
            match event {
                BackendEvent::TurnFinished {
                    outcome: yo_core::TurnOutcome::Failed(_),
                    ..
                } => break,
                BackendEvent::LocalArgumentRejectionPrepared { .. } => {
                    panic!("{case} proposed settlement")
                },
                _ => {},
            }
        }
        assert!(backend.pending_failure_context.is_none(), "{case}");
        backend.shutdown().unwrap();
    }
}

// 실제 worker·Core·disk Journal을 통과한 Failed 정산 후 cold resume의 다음 model
// request가 닫힌 tool context만 소비하고 실패 응답이나 재실행을 만들지 않습니다.
#[test]
fn local_failure_disk_resume_consumes_exact_closed_context() {
    assert_local_failure_disk_resume("tool");
}

// 실제 ask_user answered/unanswered 명령을 Core에 확정한 뒤 typed 거절로 Failed가 되어도
// disk 복구와 다음 실제 connector request가 동일한 질문 call/result를 소비합니다.
#[test]
fn local_failure_disk_resume_consumes_answered_and_unanswered_question_context() {
    for source in ["answered", "unanswered", "choice"] {
        assert_local_failure_disk_resume(source);
    }
}

fn assert_local_failure_disk_resume(source: &str) {
    use std::{
        fs,
        path::PathBuf,
        time::{Duration, Instant},
    };

    use yo_core::{
        AgentEvent, AgentIntent, AgentSession, HostWorkspacePath, SessionDescriptor,
        TranscriptRecord, TurnOutcome, WorkspaceHostId,
        session_repository::{
            LocalSessionRepository, SessionWriterRepository, recover_stored_session_continuation,
        },
    };
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let root = env::temp_dir().join(format!(
        "yo-local-failure-{}",
        WorkspaceHostId::new().unwrap()
    ));
    fs::create_dir(&root).unwrap();
    let fixture = Fixture(root);
    let storage = fixture.0.join("repository");
    let descriptor = SessionDescriptor::new(
        WorkspaceHostId::new().unwrap(),
        HostWorkspacePath::normalize_local(&fixture.0).unwrap(),
    )
    .unwrap();
    let session_id = descriptor.session_id();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let starts = Arc::new(Mutex::new(0));
    let managed = backend(
        vec![
            if source == "tool" {
                tool_call_round()
            } else {
                ordinary_question_round()
            },
            rejected_round(),
        ],
        requests.clone(),
        starts.clone(),
    );
    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
    repository.acquire_session_writer(session_id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut session =
        AgentSession::start_cancellable_with_repository(managed, descriptor, repository, || {
            Instant::now() >= deadline
        })
        .unwrap()
        .unwrap();
    let transcript = session.transcript_reader();
    let mut cursor = None;
    super::fixtures::queue_intent(&mut session, AgentIntent::submit("original input").unwrap());
    if source != "tool" {
        let deadline = Instant::now() + Duration::from_secs(2);
        let request = loop {
            session.poll().unwrap();
            let slice = transcript.read_after(None);
            if let Some(request) = slice
                .entries()
                .iter()
                .find_map(|entry| match entry.record() {
                    TranscriptRecord::EventCommitted(AgentEvent::ActivityStarted {
                        activity,
                        kind: ActivityKind::UserInputRequest { request_id },
                    }) => Some(ActivityRequestRef::new(*activity, *request_id)),
                    _ => None,
                })
            {
                break request;
            }
            assert!(Instant::now() < deadline, "ordinary question did not open");
            thread::sleep(Duration::from_millis(1));
        };
        let intent = match source {
            "answered" => AgentIntent::RespondToUserInput {
                request,
                input: "answer-canary".into(),
            },
            "choice" => AgentIntent::RespondToQuestion {
                request,
                choice: 1,
                notes: "note-canary".into(),
            },
            _ => AgentIntent::RespondToQuestionUnanswered { request },
        };
        super::fixtures::queue_intent(&mut session, intent);
    }
    super::fixtures::wait_for_turn_finish(&mut session, &transcript, &mut cursor, 1);
    assert!(
        transcript
            .read_after(None)
            .entries()
            .iter()
            .any(|entry| matches!(
                entry.record(),
                TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                    outcome: TurnOutcome::Failed(_),
                    ..
                })
            ))
    );
    session.shutdown().unwrap();
    drop(session);
    drop(transcript);
    assert_eq!(requests.lock().unwrap().len(), 2);
    let expected_starts = usize::from(source == "tool");
    assert_eq!(*starts.lock().unwrap(), expected_starts);
    if source != "tool" {
        assert_question_settlement_mutations_fail_closed(&storage, session_id);
    }
    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
    let continuation = recover_stored_session_continuation(&mut repository, session_id).unwrap();
    assert_eq!(continuation.target().model_replay().items().len(), 3);
    assert_eq!(continuation.target().epoch(), 1);
    let resumed_requests = Arc::new(Mutex::new(Vec::new()));
    let managed = backend(
        vec![completed_text_round("resumed", "done")],
        resumed_requests.clone(),
        starts.clone(),
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut session = AgentSession::start_cancellable_with_continuation(
        managed,
        continuation,
        repository,
        || Instant::now() >= deadline,
    )
    .unwrap()
    .unwrap();
    assert!(resumed_requests.lock().unwrap().is_empty());
    let transcript = session.transcript_reader();
    let mut cursor = transcript.read_after(None).head();
    super::fixtures::queue_intent(&mut session, AgentIntent::submit("next input").unwrap());
    super::fixtures::wait_for_turn_finish(&mut session, &transcript, &mut cursor, 2);
    let mut expected = if source == "tool" {
        preserved_input()
    } else {
        preserved_question_input(source)
    };
    expected.push(ModelConnectorInputItem::Message {
        role: ModelConnectorInputRole::User,
        content: "next input".into(),
        refusal: None,
    });
    assert_eq!(&resumed_requests.lock().unwrap()[0].input()[1..], expected);
    assert_eq!(*starts.lock().unwrap(), expected_starts);
    session.shutdown().unwrap();
}

struct FailedCleanupStream(Box<dyn yo_core::ModelConnectorStreamPort>);

impl yo_core::ModelConnectorStreamPort for FailedCleanupStream {
    fn poll(&mut self) -> Result<yo_core::ModelConnectorPoll, yo_core::ConnectorError> {
        self.0.poll()
    }
    fn cancel(&self) {
        self.0.cancel();
    }
    fn shutdown(&mut self) -> Result<(), yo_core::ConnectorError> {
        Err(yo_core::ConnectorError::new(
            yo_core::ConnectorFailureKind::Cleanup,
            "injected cleanup failure",
        ))
    }
}

// 이전 그룹에 완성된 문맥이 있어도 현 그룹의 실행 시도·private profile·secret barrier·
// stream 정리 실패는 로컬 정산 후보를 만들지 않습니다.
#[test]
fn local_failure_ineligible_cleanup_effect_secret_and_private_profile_stay_failed() {
    for case in [
        "effect-attempt",
        "private-profile",
        "secret-barrier",
        "cleanup-failure",
    ] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let starts = Arc::new(Mutex::new(0));
        let mut backend = backend(
            vec![tool_call_round(), rejected_round()],
            requests.clone(),
            starts.clone(),
        );
        backend
            .execute_command(AgentCommand::CreateSession {
                session_id: turn().session_id(),
            })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("original input"),
            })
            .unwrap();
        loop {
            if matches!(
                next_event(&mut backend),
                BackendEvent::ModelRequestAccepted { .. }
            ) {
                break;
            }
        }
        match case {
            "effect-attempt" => backend.turn.as_mut().unwrap().open_group_effect_attempted = true,
            "private-profile" => {
                backend.replay_profile = yo_core::ReplayProfile::ProviderPrivateLocalPlaintext
            },
            "secret-barrier" => backend.turn.as_mut().unwrap().terminal_secret_request = true,
            "cleanup-failure" => {
                let state = backend.turn.as_mut().unwrap();
                state.stream = Some(Box::new(FailedCleanupStream(state.stream.take().unwrap())));
            },
            _ => unreachable!(),
        }
        loop {
            match next_event(&mut backend) {
                BackendEvent::TurnFinished {
                    outcome: yo_core::TurnOutcome::Failed(_),
                    ..
                } => break,
                BackendEvent::LocalArgumentRejectionPrepared { .. } => {
                    panic!("{case} proposed settlement")
                },
                _ => {},
            }
        }
        assert!(backend.pending_failure_context.is_none(), "{case}");
        assert_eq!(*starts.lock().unwrap(), 1);
        assert_eq!(requests.lock().unwrap().len(), 2);
        backend.shutdown().unwrap();
    }
}

// 여러 응답 묶음의 누적 context는 마지막 call/result 쌍만으로 축약하지 않고 다음
// model request가 두 묶음을 정확한 순서와 원문으로 한 번씩 소비합니다.
#[test]
fn local_failure_preserves_every_closed_current_turn_group() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let starts = Arc::new(Mutex::new(0));
    let second = tool_call_round()
        .into_iter()
        .map(|event| match event {
            ModelConnectorEvent::ResponseCreated { .. } => ModelConnectorEvent::ResponseCreated {
                response_id: "tool-2".into(),
            },
            ModelConnectorEvent::FunctionCallStarted {
                output_index, name, ..
            } => ModelConnectorEvent::FunctionCallStarted {
                output_index,
                name,
                item_id: "item-2".into(),
                call_id: "call-2".into(),
            },
            ModelConnectorEvent::FunctionCallDone {
                output_index,
                name,
                arguments,
                ..
            } => ModelConnectorEvent::FunctionCallDone {
                output_index,
                name,
                arguments,
                item_id: "item-2".into(),
                call_id: "call-2".into(),
            },
            ModelConnectorEvent::Terminal { status, usage, .. } => ModelConnectorEvent::Terminal {
                response_id: "tool-2".into(),
                status,
                usage,
            },
            _ => unreachable!(),
        })
        .collect();
    let mut backend = backend(
        vec![
            tool_call_round(),
            second,
            rejected_round(),
            completed_text_round("next", "done"),
        ],
        requests.clone(),
        starts.clone(),
    );
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("original input"),
        })
        .unwrap();
    loop {
        if let BackendEvent::LocalArgumentRejectionPrepared {
            replay: Some(delta),
            ..
        } = next_event(&mut backend)
        {
            assert_eq!(delta.items().len(), 5);
            break;
        }
    }
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn_number(2),
            input: UserInput::from("next input"),
        })
        .unwrap();
    let mut expected = preserved_input();
    expected.extend([
        ModelConnectorInputItem::FunctionCall {
            call_id: "call-2".into(),
            name: "read_file".into(),
            arguments: r#"{"path":"README.md"}"#.into(),
        },
        ModelConnectorInputItem::FunctionCallOutput {
            call_id: "call-2".into(),
            output: r#"{"contents":"ok"}"#.into(),
        },
        ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: "next input".into(),
            refusal: None,
        },
    ]);
    assert_eq!(&requests.lock().unwrap()[3].input()[1..], expected);
    assert_eq!(*starts.lock().unwrap(), 2);
    backend.shutdown().unwrap();
}

// Core가 검증하는 checkpoint 반환 경계를 주입해 backend 소비자를 분리합니다.
// 빈 후속 suffix는 delta를 생략하고 steer-only suffix는 그 입력만 추가하여 root를 중복하지
// 않습니다.
#[test]
fn local_failure_checkpoint_consumer_preserves_empty_and_steering_only_suffixes() {
    use yo_core::{ModelReplay, ModelReplayItem, ModelReplayRole};

    use crate::backend::CompactionState;
    for with_steer in [false, true] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let starts = Arc::new(Mutex::new(0));
        let mut backend = backend(
            vec![
                tool_call_round(),
                rejected_round(),
                completed_text_round("next", "done"),
            ],
            requests.clone(),
            starts,
        );
        backend
            .execute_command(AgentCommand::CreateSession {
                session_id: turn().session_id(),
            })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("original input"),
            })
            .unwrap();
        loop {
            if matches!(
                next_event(&mut backend),
                BackendEvent::ContextActiveSuffixCompleted { .. }
            ) {
                break;
            }
        }
        let body = super::fixtures::portable_summary();
        let mut root = vec![ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: body.clone(),
            refusal: None,
        }];
        root.extend(backend.turn.as_ref().unwrap().delta.clone());
        let replay = ModelReplay::from_checkpoint(backend.contract.clone(), root).unwrap();
        let state = backend.turn.as_mut().unwrap();
        state.start_next_round = false;
        state.compaction = Some(CompactionState::AwaitingCheckpoint {
            replay,
            included_steers: 0,
            included_steer_encoded_bytes: 0,
        });
        if with_steer {
            backend
                .execute_command(AgentCommand::SteerTurn {
                    turn: turn(),
                    input: UserInput::from("checkpoint correction"),
                })
                .unwrap();
            backend.commit_prepared_command().unwrap();
        }
        loop {
            if let BackendEvent::LocalArgumentRejectionPrepared { replay, .. } =
                next_event(&mut backend)
            {
                if with_steer {
                    let delta = replay.unwrap();
                    assert!(delta.contract().is_none());
                    assert_eq!(
                        delta.items(),
                        [UserInput::from("checkpoint correction").model_replay_item()]
                    );
                } else {
                    assert!(replay.is_none());
                }
                break;
            }
        }
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn_number(2),
                input: UserInput::from("next input"),
            })
            .unwrap();
        let mut expected = vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: body,
            refusal: None,
        }];
        expected.extend(preserved_input());
        if with_steer {
            expected.push(ModelConnectorInputItem::Message {
                role: ModelConnectorInputRole::User,
                content: "checkpoint correction".into(),
                refusal: None,
            });
        }
        expected.push(ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: "next input".into(),
            refusal: None,
        });
        assert_eq!(&requests.lock().unwrap()[2].input()[1..], expected);
        backend.shutdown().unwrap();
    }
}

struct RejectSettlementRepository(LocalSessionRepository);

impl SessionRepository for RejectSettlementRepository {
    fn append(
        &mut self,
        session: yo_core::SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        use yo_core::session_repository::{AppendError, RepositoryError};
        if record.payload().contains("yo.local-failure-context/v1") {
            return Err(AppendError::Repository(RepositoryError::Unavailable {
                message: "injected settlement append failure".into(),
            }));
        }
        self.0.append(session, record)
    }
    fn read_after(
        &self,
        session: yo_core::SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        self.0.read_after(session, sequence, limit)
    }
}
impl SessionWriterRepository for RejectSettlementRepository {
    fn acquire_session_writer(
        &mut self,
        session: yo_core::SessionId,
    ) -> Result<(), RepositoryError> {
        self.0.acquire_session_writer(session)
    }
}

// 실제 managed 후보의 원자적 disk 추기를 거절하면 worker가 후보를 정지하며 다음
// 제출은 model에 도달하지 않습니다. Cold recovery도 이전 Anchor로 되돌아가지 않습니다.
#[test]
fn local_failure_managed_append_failure_stops_pending_context_and_next_dispatch() {
    use std::{
        fs,
        path::PathBuf,
        thread,
        time::{Duration, Instant},
    };

    use yo_core::{
        AgentIntent, AgentSession, HostWorkspacePath, SessionDescriptor, WorkspaceHostId,
        session_repository::{
            LocalSessionRepository, SessionWriterRepository, recover_stored_session_continuation,
        },
    };
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let root = env::temp_dir().join(format!(
        "yo-local-failure-append-{}",
        WorkspaceHostId::new().unwrap()
    ));
    fs::create_dir(&root).unwrap();
    let fixture = Fixture(root);
    let storage = fixture.0.join("repository");
    let descriptor = SessionDescriptor::new(
        WorkspaceHostId::new().unwrap(),
        HostWorkspacePath::normalize_local(&fixture.0).unwrap(),
    )
    .unwrap();
    let session_id = descriptor.session_id();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let starts = Arc::new(Mutex::new(0));
    let managed = backend(
        vec![tool_call_round(), rejected_round()],
        requests.clone(),
        starts.clone(),
    );
    let mut repository = RejectSettlementRepository(
        LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap(),
    );
    repository.acquire_session_writer(session_id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut session =
        AgentSession::start_cancellable_with_repository(managed, descriptor, repository, || {
            Instant::now() >= deadline
        })
        .unwrap()
        .unwrap();
    super::fixtures::queue_intent(&mut session, AgentIntent::submit("original input").unwrap());
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if session.poll().is_err() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "append rejection did not stop the worker"
        );
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        session
            .dispatch(AgentIntent::submit("must not dispatch").unwrap())
            .is_err()
    );
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert_eq!(*starts.lock().unwrap(), 1);
    let _ = session.shutdown();
    drop(session);
    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
    assert!(recover_stored_session_continuation(&mut repository, session_id).is_err());
}

// 정산 반환 뒤 추가 poll 없이 /compact가 먼저 도착해도 보존 그룹을 승격합니다.
// 두 그룹을 가진 실제 summary 요청은 앞선 완료 Turn만 요약하고 실패 suffix를 보존합니다.
#[test]
fn local_failure_direct_compaction_promotes_committed_context_before_request() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let starts = Arc::new(Mutex::new(0));
    let mut backend = backend(
        vec![
            completed_text_round("baseline", "baseline answer"),
            tool_call_round(),
            rejected_round(),
            completed_text_round("summary", &super::fixtures::portable_summary()),
        ],
        requests.clone(),
        starts.clone(),
    );
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: turn().session_id(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("baseline input"),
        })
        .unwrap();
    loop {
        if matches!(
            next_event(&mut backend),
            BackendEvent::ResumableTurnFinished { .. }
        ) {
            break;
        }
    }
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn_number(2),
            input: UserInput::from("original input"),
        })
        .unwrap();
    loop {
        if matches!(
            next_event(&mut backend),
            BackendEvent::LocalArgumentRejectionPrepared { .. }
        ) {
            break;
        }
    }
    assert_eq!(
        backend.replay_groups.len(),
        1,
        "pending context promoted early"
    );
    backend
        .execute_command(AgentCommand::CompactContext { guidance: None })
        .unwrap();
    assert_eq!(backend.replay_groups.len(), 2);
    assert!(backend.pending_failure_context.is_none());
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests[3].tools().is_none());
    let ModelConnectorInputItem::Message {
        role: ModelConnectorInputRole::User,
        content,
        ..
    } = &requests[3].input()[1]
    else {
        panic!("summary must use inert user history")
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(content).unwrap(),
        serde_json::json!({"history":[
            {"type":"message","role":"user","content":"baseline input","refusal":null},
            {"type":"message","role":"assistant","content":"baseline answer","refusal":null},
        ]})
    );
    assert_eq!(*starts.lock().unwrap(), 1);
    drop(requests);
    backend.shutdown().unwrap();
}
