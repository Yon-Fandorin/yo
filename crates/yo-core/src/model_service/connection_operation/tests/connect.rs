use std::fs;

use super::{
    super::connect::ConnectStep,
    support::{CANDIDATE_SECRET, Fixture, account, candidate, provider},
};
use crate::model_service::{
    CompleteModelBinding, ConnectionAccount, ConnectionOperationExecutionError,
    ConnectionOperationExecutionOutcome, ConnectionOperationPhase,
    LocalConnectionOperationRepositories, StartupTarget, StoredModelBinding,
};

// runtime이 지원하지 않는 resolved profile은 TTY prompt나 remote request보다 앞선 prepare에서
// 실패하고 세 저장소를 만들지 않아 쓸 수 없는 profile에 secret을 요구하지 않습니다.
#[test]
fn unsupported_profile_fails_during_secret_free_preparation() {
    let fixture = Fixture::new("external-profile-admission");
    let repositories = repositories(&fixture);
    let mut session = repositories.acquire().unwrap();
    let binding = unsupported_complete();
    let connection = direct_connect_mutation(&fixture, &binding);

    let error = session
        .prepare_external_connection(connection, vec![binding])
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
        let connection = direct_connect_mutation(&fixture, &binding);

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
        Some(&model_target("alpha"))
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
                Some(&model_target("alpha"))
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
    let connection = direct_connect_mutation(
        fixture,
        bindings
            .first()
            .expect("test direct connects contain one selected model"),
    );
    session
        .prepare_external_connection(connection, bindings)
        .unwrap()
}

fn direct_connect_mutation(
    fixture: &Fixture,
    binding: &CompleteModelBinding,
) -> crate::model_service::PreparedConnectionMutation {
    let coordinate = binding.binding();
    let account = ConnectionAccount::new(
        coordinate.provider_id().clone(),
        coordinate.account_id().clone(),
        None,
        None,
    )
    .unwrap();
    fixture
        .connections
        .capture()
        .unwrap()
        .prepare_model_connect(
            account,
            StoredModelBinding::new(binding.clone(), None).unwrap(),
        )
        .unwrap()
}

// The public mutation and the credential-changing definition must describe the same exact pair.
// An empty arbitrary mutation cannot be relabeled as a catalog-only import.
#[test]
fn definition_preparation_rejects_a_public_mutation_without_the_exact_pair() {
    let fixture = Fixture::new("definition-public-mismatch");
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
        .prepare_external_definition(connection, &provider(), &account(), Vec::new())
        .err()
        .expect("an arbitrary public mutation must not become a definition import");

    assert!(
        error
            .to_string()
            .contains("exact stored Provider-and-Account definition")
    );
    assert!(fixture.journal.capture().unwrap().is_none());
    assert!(fixture.credentials.capture().unwrap().is_empty());
}

// 최종 bytes에 대상 pair가 우연히 남아 있어도 다른 pair의 group replacement를 대상
// credential 회전으로 다시 이름 붙일 수 없습니다.
#[test]
fn definition_preparation_rejects_an_unrelated_group_replacement() {
    let fixture = Fixture::new("definition-unrelated-group");
    let target = complete("alpha");
    let initial = direct_connect_mutation(&fixture, &target);
    fixture.connections.commit(&initial).unwrap();

    let other = complete_at("vendor", "other", "beta");
    let other_account = ConnectionAccount::new(
        other.binding().provider_id().clone(),
        other.binding().account_id().clone(),
        None,
        None,
    )
    .unwrap();
    let unrelated = fixture
        .connections
        .capture()
        .unwrap()
        .prepare_group_replace(
            other_account,
            vec![StoredModelBinding::new(other, None).unwrap()],
            None,
        )
        .unwrap();
    let repositories = repositories(&fixture);
    let mut session = repositories.acquire().unwrap();

    let error = session
        .prepare_external_definition(unrelated, &provider(), &account(), vec![target])
        .err()
        .expect("another pair's replacement must not rotate this pair's credential");

    assert!(
        error
            .to_string()
            .contains("exact stored Provider-and-Account definition")
    );
    assert!(fixture.journal.capture().unwrap().is_none());
    assert!(fixture.credentials.capture().unwrap().is_empty());
}

// Direct-connect admission is bound to prepare_model_connect itself. Group replacement and
// preference mutations cannot borrow that credential-changing lane merely because their final
// bytes retain the complete bindings supplied by a caller.
#[test]
fn direct_preparation_rejects_non_connect_mutation_intents() {
    let fixture = Fixture::new("direct-connect-intent");
    let alpha = complete("alpha");
    let beta = complete("beta");
    let stored_alpha = StoredModelBinding::new(alpha.clone(), None).unwrap();
    let stored_beta = StoredModelBinding::new(beta.clone(), None).unwrap();
    let pair_account = ConnectionAccount::new(provider(), account(), None, None).unwrap();
    let initial = fixture
        .connections
        .capture()
        .unwrap()
        .prepare_group_replace(
            pair_account.clone(),
            vec![stored_alpha.clone(), stored_beta],
            None,
        )
        .unwrap();
    fixture.connections.commit(&initial).unwrap();
    let snapshot = fixture.connections.capture().unwrap();
    let preference_only = snapshot
        .prepare_preference(Some(StartupTarget::Model(stored_alpha.selection())))
        .unwrap()
        .unwrap();
    let deleting_group = snapshot
        .prepare_group_replace(pair_account, vec![stored_alpha], None)
        .unwrap();
    let repositories = repositories(&fixture);
    let mut session = repositories.acquire().unwrap();

    for (mutation, bindings) in [
        (preference_only, vec![alpha.clone(), beta]),
        (deleting_group, vec![alpha]),
    ] {
        let error = session
            .prepare_external_connection(mutation, bindings)
            .err()
            .expect("only a direct-connect mutation may rotate its credential");
        assert!(
            error
                .to_string()
                .contains("exact stored Provider-and-Account definition")
        );
    }
    assert!(fixture.journal.capture().unwrap().is_none());
    assert!(fixture.credentials.capture().unwrap().is_empty());
}

fn complete(model: &str) -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(&format!(
        r#"{{"provider":"qwencloud","account":"default","model":"{model}","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{}},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1"}}"#
    ))
    .unwrap()
}

fn model_target(model: &str) -> StartupTarget {
    StartupTarget::Model(
        StoredModelBinding::new(complete(model), None)
            .unwrap()
            .selection(),
    )
}

fn complete_at(provider: &str, account: &str, model: &str) -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(&format!(
        r#"{{"provider":"{provider}","account":"{account}","model":"{model}","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{}},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1"}}"#,
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
