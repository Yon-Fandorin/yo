use std::fs;

use super::{
    super::connect::ConnectStep,
    support::{CANDIDATE_SECRET, Fixture, account, candidate, provider},
};
use crate::model_service::{
    CompleteModelBinding, ConnectionOperationExecutionError, ConnectionOperationExecutionOutcome,
    ConnectionOperationPhase, LocalConnectionOperationRepositories, StartupTarget,
};

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
        .prepare_external_connection(connection, vec![unsupported_complete()])
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

// replay profile과 Provider-specific Kimi envelope는 실제 model use까지 미루지 않고
// secret-free preparation에서 닫아 preview나 credential 입력으로 진행하지 않습니다.
#[test]
fn complete_binding_admission_precedes_external_publication() {
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
            .prepare_external_connection(connection, vec![binding])
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
fn captured_candidate_commits_credential_then_public_and_clears_journal() {
    let fixture = Fixture::new("external-commit");
    let repositories = repositories(&fixture);
    let mut session = repositories.acquire().unwrap();
    let prepared = prepared(&mut session, &fixture, vec![complete("alpha")]);

    session
        .commit_external_connection(prepared, candidate())
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

        assert!(matches!(
            session.commit_external_connection_until(prepared, candidate(), step),
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
        .prepare_external_connection(connection, bindings)
        .unwrap()
}

fn complete(model: &str) -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(&format!(
        r#"{{"provider":"qwencloud","account":"default","model":"{model}","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{}},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1"}}"#
    ))
    .unwrap()
}

fn unsupported_complete() -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(
        r#"{"provider":"qwencloud","account":"default","model":"unsupported","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{},"optional_request_parameters":{"temperature":1.0},"tool_capability_policy":"local-tools/v1"}"#,
    )
    .unwrap()
}

fn cross_dialect_private_complete() -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(
        r#"{"provider":"qwencloud","account":"default","model":"private-generic","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","replay_profile":"kimi-private-local-plaintext/v1"}"#,
    )
    .unwrap()
}

fn invalid_kimi_complete() -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(
        r#"{"provider":"kimi","account":"default","model":"kimi-k2.6","connector":"kimi-chat-completions","base_url":"https://api.moonshot.ai/v1","api_dialect":"kimi-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":262144,"max_output_tokens":32767,"reasoning_parameters":{},"optional_request_parameters":{"thinking":{"type":"disabled"}},"tool_capability_policy":"local-tools/v1"}"#,
    )
    .unwrap()
}

fn cross_product_kimi_complete() -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(
        r#"{"provider":"kimi","account":"default","model":"kimi-k3","connector":"kimi-chat-completions","base_url":"https://api.kimi.com/coding/v1","api_dialect":"kimi-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1048576,"max_output_tokens":131072,"reasoning_parameters":{"effort":"max"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","replay_profile":"kimi-private-local-plaintext/v1"}"#,
    )
    .unwrap()
}
