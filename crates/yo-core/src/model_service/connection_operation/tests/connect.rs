use std::{
    cell::RefCell,
    collections::VecDeque,
    fs,
    time::{Duration, Instant},
};

use super::{
    super::connect::{
        ConnectStep, VerificationStreamPort, verification_limits, verification_request,
        verification_request_with_cache, verify_external_connection_for_test,
        verify_semantic_stream,
    },
    support::{CANDIDATE_SECRET, Fixture, account, candidate, digest, provider},
};
use crate::{
    ConnectorFailureKind, ModelConnectorEvent, ModelConnectorPoll, ModelConnectorTerminal,
    ModelConnectorUsage,
    model_connector::ModelCacheAffinityHint,
    model_service::{
        CompleteModelBinding, ConnectionOperationExecutionError,
        ConnectionOperationExecutionOutcome, ConnectionOperationPhase,
        LocalConnectionOperationRepositories, StartupTarget,
    },
};

// interactive agent와 달리 connection verification은 explicit 10분 absolute deadline을
// 제공하면서 같은 finite transport phase 기본값을 유지하는지 고정합니다.
#[test]
fn connection_verification_supplies_the_non_agent_absolute_deadline() {
    let limits = verification_limits();

    assert_eq!(
        limits.absolute_request_timeout,
        Some(Duration::from_secs(10 * 60))
    );
    assert_eq!(limits.connect_timeout, Duration::from_secs(30));
    assert_eq!(limits.response_header_timeout, Duration::from_secs(5 * 60));
    assert_eq!(limits.stream_idle_timeout, Duration::from_secs(5 * 60));
    assert_eq!(limits.error_body_idle_timeout, Duration::from_secs(30));
    assert_eq!(limits.event_delivery_timeout, Duration::from_secs(5 * 60));
}

// binding의 durable policy가 local-tools여도 connection verification은 request-local
// exposure를 disabled로 고정해 두 Connector wire에서 현재 tools를 보내지 않습니다.
#[test]
fn connection_verification_always_disables_current_tool_exposure() {
    let request = verification_request(&complete("verified")).unwrap();
    let body = request.tokenization_payload("verified");

    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
}

// Kimi connector는 profile의 output cap과 wire cap이 정확히 같아야 하므로 검증 요청도
// generic 32-token 축약 대신 해당 complete profile의 값을 그대로 사용합니다.
#[test]
fn connection_verification_preserves_the_exact_kimi_output_cap() {
    let request = verification_request(&kimi_complete()).unwrap();
    assert_eq!(
        request.tokenization_payload("kimi-k2.6")["max_output_tokens"],
        32_768
    );
}

// Code verification은 Session이 없으므로 service-owned verification UUID hint를 사용하고,
// exact Code Connector body에서만 verification- prefix의 opaque key가 직렬화됩니다.
#[test]
fn code_verification_uses_one_ephemeral_typed_cache_affinity() {
    let task = crate::fixture_session(41);
    let hint = ModelCacheAffinityHint::for_verification(task);
    let request = verification_request_with_cache(&kimi_code_complete(), Some(&hint)).unwrap();

    let expected = format!("verification-{task}");
    assert_eq!(request.cache_affinity_hint(), Some(expected.as_str()));
    let diagnostic = format!("{request:?}");
    assert!(diagnostic.contains("ModelCacheAffinityHint([redacted])"));
    assert!(!diagnostic.contains(&format!("verification-{task}")));
}

struct FakeVerificationStream {
    polls: VecDeque<Result<ModelConnectorPoll, ConnectorFailureKind>>,
    cancelled: bool,
    shutdown: bool,
}

impl VerificationStreamPort for FakeVerificationStream {
    fn poll(&mut self) -> Result<ModelConnectorPoll, ConnectorFailureKind> {
        self.polls
            .pop_front()
            .unwrap_or(Ok(ModelConnectorPoll::Closed))
    }

    fn cancel(&mut self) {
        self.cancelled = true;
    }

    fn shutdown(&mut self) -> Result<(), ConnectorFailureKind> {
        self.shutdown = true;
        Ok(())
    }
}

// 검증 stream은 MessageDone과 Completed terminal이 모두 있어야 성공하고 completed refusal도
// 유효한 assistant 결과로 받되 terminal-only 응답은 실패하며, 모든 경로가 cleanup을 실행합니다.
#[test]
fn semantic_verification_requires_message_completion_and_always_cleans_up() {
    let completed = ModelConnectorPoll::Event(ModelConnectorEvent::Terminal {
        response_id: "response".to_owned(),
        status: ModelConnectorTerminal::Completed,
        usage: ModelConnectorUsage::default(),
    });
    let message = ModelConnectorPoll::Event(ModelConnectorEvent::MessageDone {
        output_index: 0,
        item_id: "message".to_owned(),
    });
    let refusal = ModelConnectorPoll::Event(ModelConnectorEvent::RefusalDelta {
        output_index: 0,
        item_id: "message".to_owned(),
        content_index: 0,
        delta: "no".to_owned(),
    });
    for (polls, expected) in [
        (vec![Ok(message.clone()), Ok(completed.clone())], Ok(())),
        (
            vec![Ok(completed.clone())],
            Err(ConnectorFailureKind::Protocol),
        ),
        (
            vec![Ok(refusal), Ok(message.clone()), Ok(completed.clone())],
            Ok(()),
        ),
        (
            vec![Err(ConnectorFailureKind::Transport)],
            Err(ConnectorFailureKind::Transport),
        ),
    ] {
        let mut stream = FakeVerificationStream {
            polls: polls.into(),
            cancelled: false,
            shutdown: false,
        };

        assert_eq!(
            verify_semantic_stream(&mut stream, Instant::now() + Duration::from_secs(1)),
            expected
        );
        assert!(stream.cancelled);
        assert!(stream.shutdown);
    }
}

// candidate 검증은 같은 Account에 남는 모든 complete binding을 순서대로 방문하고 하나라도
// 실패하면 journal, credential, public 저장소를 만들지 않아 부분 검증을 성공으로 오인하지 않습니다.
#[test]
fn candidate_must_verify_every_retained_binding_before_any_write() {
    let fixture = Fixture::new("external-verify-all");
    let repositories = repositories(&fixture);
    let mut session = repositories.acquire().unwrap();
    let prepared = prepared(
        &mut session,
        &fixture,
        vec![complete("alpha"), complete("beta")],
    );
    let visited = RefCell::new(Vec::new());

    let error =
        verify_external_connection_for_test(prepared, candidate(), |binding, credential| {
            assert_eq!(credential.expose_secret(), CANDIDATE_SECRET);
            let model = binding.binding().model_id().as_str().to_owned();
            visited.borrow_mut().push(model.clone());
            if model == "beta" {
                Err(ConnectorFailureKind::HttpStatus)
            } else {
                Ok(())
            }
        })
        .err()
        .expect("the second binding must reject the candidate");

    assert_eq!(visited.into_inner(), ["alpha", "beta"]);
    assert!(error.to_string().contains("qwencloud:default:beta"));
    assert!(!error.to_string().contains(CANDIDATE_SECRET));
    assert!(fixture.journal.capture().unwrap().is_none());
    assert!(
        fixture
            .credentials
            .capture()
            .unwrap()
            .revision()
            .is_absent()
    );
    assert!(
        fixture
            .connections
            .capture()
            .unwrap()
            .revision()
            .is_absent()
    );
}

// runtime이 지원하지 않는 resolved profile은 TTY prompt나 remote request보다 앞선 prepare에서
// 실패하고 세 저장소를 만들지 않아 쓸 수 없는 profile에 secret을 요구하지 않습니다.
#[test]
fn unsupported_profile_fails_during_secret_free_preparation() {
    let fixture = Fixture::new("external-profile-admission");
    let repositories = repositories(&fixture);
    let mut session = repositories.acquire().unwrap();
    let connection = fixture
        .connections
        .capture()
        .unwrap()
        .prepare_preference(Some(StartupTarget::HostCodex))
        .unwrap()
        .unwrap();

    let error = session
        .prepare_external_connection(digest('f'), connection, vec![unsupported_complete()])
        .err()
        .expect("unsupported optional parameters must fail preparation");

    assert!(
        error
            .to_string()
            .contains("does not support the resolved profile")
    );
    assert!(fixture.journal.capture().unwrap().is_none());
    assert!(
        fixture
            .credentials
            .capture()
            .unwrap()
            .revision()
            .is_absent()
    );
    assert!(
        fixture
            .connections
            .capture()
            .unwrap()
            .revision()
            .is_absent()
    );
}

// replay profile과 Provider-specific Kimi envelope는 remote verification까지 미루지 않고
// secret-free preparation에서 닫아 preview나 credential 입력으로 진행하지 않습니다.
#[test]
fn complete_binding_admission_precedes_external_verification() {
    for binding in [
        cross_dialect_private_complete(),
        invalid_kimi_complete(),
        cross_product_kimi_complete(),
    ] {
        let fixture = Fixture::new("complete-binding-preflight");
        let repositories = repositories(&fixture);
        let mut session = repositories.acquire().unwrap();
        let connection = fixture
            .connections
            .capture()
            .unwrap()
            .prepare_preference(Some(StartupTarget::HostCodex))
            .unwrap()
            .unwrap();

        let error = session
            .prepare_external_connection(digest('f'), connection, vec![binding])
            .err()
            .expect("invalid complete binding must fail preparation");

        assert!(
            error
                .to_string()
                .contains("does not support the resolved profile")
        );
        assert!(fixture.journal.capture().unwrap().is_none());
        assert!(
            fixture
                .credentials
                .capture()
                .unwrap()
                .revision()
                .is_absent()
        );
        assert!(
            fixture
                .connections
                .capture()
                .unwrap()
                .revision()
                .is_absent()
        );
    }
}

// 정상 connect는 secret-free intent 뒤 credential을 먼저 저장하고 exact public mutation을
// 게시한 다음 Complete journal을 지워, 최종 세 저장소가 한 operation으로 수렴합니다.
#[test]
fn verified_candidate_commits_credential_then_public_and_clears_journal() {
    let fixture = Fixture::new("external-commit");
    let repositories = repositories(&fixture);
    let mut session = repositories.acquire().unwrap();
    let prepared = prepared(&mut session, &fixture, vec![complete("alpha")]);
    let verified =
        verify_external_connection_for_test(prepared, candidate(), |_, _| Ok(())).unwrap();

    session
        .commit_verified_external_connection(verified)
        .unwrap();

    assert_eq!(
        fixture
            .credentials
            .capture()
            .unwrap()
            .resolve(&provider(), &account())
            .unwrap()
            .expose_secret(),
        CANDIDATE_SECRET
    );
    assert_eq!(
        fixture.connections.capture().unwrap().preference(),
        Some(&StartupTarget::HostCodex)
    );
    assert!(fixture.journal.capture().unwrap().is_none());
}

// 각 durable effect 직후 중단해도 journal에는 candidate가 없고 새 session의 recovery가
// expected/expected intent는 abandon, credential 이후 절단점은 exact public 상태로 수렴합니다.
#[test]
fn every_connect_effect_cut_is_secret_free_and_recoverable() {
    for step in [
        ConnectStep::JournalPublished,
        ConnectStep::CredentialCommitted,
        ConnectStep::JournalAdvanced(ConnectionOperationPhase::CredentialCommitted),
        ConnectStep::PublicCommitted,
        ConnectStep::JournalAdvanced(ConnectionOperationPhase::PublicCommitted),
        ConnectStep::JournalAdvanced(ConnectionOperationPhase::Complete),
        ConnectStep::JournalCleared,
    ] {
        let fixture = Fixture::new(&format!("external-cut-{step:?}"));
        let repositories = repositories(&fixture);
        let mut session = repositories.acquire().unwrap();
        let prepared = prepared(&mut session, &fixture, vec![complete("alpha")]);
        let verified =
            verify_external_connection_for_test(prepared, candidate(), |_, _| Ok(())).unwrap();

        assert!(matches!(
            session.commit_verified_external_connection_until(verified, step),
            Err(ConnectionOperationExecutionError::InjectedInterruption)
        ));
        drop(session);
        if fixture.journal.path().exists() {
            let journal = fs::read_to_string(fixture.journal.path()).unwrap();
            assert!(!journal.contains(CANDIDATE_SECRET));
        }

        let outcome = repositories
            .acquire()
            .unwrap()
            .recover_pending_operation()
            .unwrap();
        if step == ConnectStep::JournalPublished {
            assert!(matches!(
                outcome,
                ConnectionOperationExecutionOutcome::Abandoned { .. }
            ));
            assert!(
                fixture
                    .credentials
                    .capture()
                    .unwrap()
                    .revision()
                    .is_absent()
            );
            assert!(
                fixture
                    .connections
                    .capture()
                    .unwrap()
                    .revision()
                    .is_absent()
            );
        } else {
            assert!(matches!(
                outcome,
                ConnectionOperationExecutionOutcome::Completed { .. }
                    | ConnectionOperationExecutionOutcome::NoPendingOperation
            ));
            assert!(
                fixture
                    .credentials
                    .capture()
                    .unwrap()
                    .resolve(&provider(), &account())
                    .is_some()
            );
            assert_eq!(
                fixture.connections.capture().unwrap().preference(),
                Some(&StartupTarget::HostCodex)
            );
        }
        assert!(fixture.journal.capture().unwrap().is_none());
    }
}

fn repositories(fixture: &Fixture) -> LocalConnectionOperationRepositories {
    LocalConnectionOperationRepositories::from_paths(
        fixture.connections.path(),
        fixture.credentials.path(),
        fixture.journal.path(),
    )
    .unwrap()
}

fn prepared(
    session: &mut crate::model_service::LocalConnectionOperationSession<'_>,
    fixture: &Fixture,
    bindings: Vec<CompleteModelBinding>,
) -> crate::model_service::PreparedExternalConnection {
    let connection = fixture
        .connections
        .capture()
        .unwrap()
        .prepare_preference(Some(StartupTarget::HostCodex))
        .unwrap()
        .unwrap();
    session
        .prepare_external_connection(digest('f'), connection, bindings)
        .unwrap()
}

fn complete(model: &str) -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(&format!(
        r#"{{"provider":"qwencloud","account":"default","model":"{model}","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{}},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1"}}"#
    ))
    .unwrap()
}

fn unsupported_complete() -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(
        r#"{"provider":"qwencloud","account":"default","model":"unsupported","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{},"optional_request_parameters":{"temperature":1.0},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1"}"#,
    )
    .unwrap()
}

fn cross_dialect_private_complete() -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(
        r#"{"provider":"qwencloud","account":"default","model":"private-generic","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1","replay_profile":"kimi-private-local-plaintext/v1"}"#,
    )
    .unwrap()
}

fn invalid_kimi_complete() -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(
        r#"{"provider":"kimi","account":"default","model":"kimi-k2.6","connector":"kimi-chat-completions","base_url":"https://api.moonshot.ai/v1","api_dialect":"kimi-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":262144,"max_output_tokens":32767,"reasoning_parameters":{},"optional_request_parameters":{"thinking":{"type":"disabled"}},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1"}"#,
    )
    .unwrap()
}

fn cross_product_kimi_complete() -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(
        r#"{"provider":"kimi","account":"default","model":"kimi-k3","connector":"kimi-chat-completions","base_url":"https://api.kimi.com/coding/v1","api_dialect":"kimi-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1048576,"max_output_tokens":131072,"reasoning_parameters":{"effort":"max"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1","replay_profile":"kimi-private-local-plaintext/v1"}"#,
    )
    .unwrap()
}

fn kimi_complete() -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(
        r#"{"provider":"kimi","account":"default","model":"kimi-k2.6","connector":"kimi-chat-completions","base_url":"https://api.moonshot.ai/v1","api_dialect":"kimi-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":262144,"max_output_tokens":32768,"reasoning_parameters":{},"optional_request_parameters":{"thinking":{"type":"disabled"}},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1"}"#,
    )
    .unwrap()
}

fn kimi_code_complete() -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(
        r#"{"provider":"kimi","account":"default","model":"k3-256k","connector":"kimi-chat-completions","base_url":"https://api.kimi.com/coding/v1","api_dialect":"kimi-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":262144,"max_output_tokens":131072,"reasoning_parameters":{"effort":"high"},"optional_request_parameters":{"thinking":{"type":"enabled","keep":"all"}},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1","replay_profile":"kimi-private-local-plaintext/v1"}"#,
    )
    .unwrap()
}
