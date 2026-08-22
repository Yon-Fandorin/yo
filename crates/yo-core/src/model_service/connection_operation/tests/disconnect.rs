use super::{
    super::disconnect::{DisconnectStep, ExternalDisconnectCredentialAction},
    support::{Fixture, account, candidate, provider},
};
use crate::model_service::{
    ApiCredential, ConnectionAccount, ConnectionOperationExecutionError,
    ConnectionOperationExecutionOutcome, ConnectionOperationPhase,
    LocalConnectionOperationRepositories, ModelId, ModelSelection, StartupTarget,
    StoredModelBinding,
};

// remove disconnect는 secret-free intent 뒤 public state를 먼저 게시하고 credential을 지운 뒤
// journal을 clear하여 새 startup이 제거된 binding을 보는 동안 낡은 secret만 남는 방향으로
// 수렴합니다.
#[test]
fn remove_commits_public_before_credential_and_clears_journal() {
    let fixture = Fixture::new("disconnect-remove");
    seed_pair(&fixture);
    let repositories = repositories(&fixture);
    let mut session = repositories.acquire().unwrap();
    let prepared = prepared(
        &mut session,
        &fixture,
        ExternalDisconnectCredentialAction::Remove,
    );

    session.commit_external_disconnect(prepared).unwrap();

    assert!(
        fixture
            .connections
            .capture()
            .unwrap()
            .preference()
            .is_none()
    );
    assert!(
        fixture
            .credentials
            .capture()
            .unwrap()
            .resolve(&provider(), &account())
            .is_none()
    );
    assert!(fixture.journal.capture().unwrap().is_none());
}

// preserve disconnect는 같은 public-first 순서를 사용하지만 credential CAS를 만들지 않아
// 남은 stored model이 사용하는 exact Provider/Account secret을 그대로 보존합니다.
#[test]
fn preserve_commits_public_without_mutating_credential() {
    let fixture = Fixture::new("disconnect-preserve");
    seed_pair(&fixture);
    let before = fixture.credentials.capture().unwrap().revision().clone();
    let repositories = repositories(&fixture);
    let mut session = repositories.acquire().unwrap();
    let prepared = prepared(
        &mut session,
        &fixture,
        ExternalDisconnectCredentialAction::Preserve,
    );

    session.commit_external_disconnect(prepared).unwrap();

    assert!(
        fixture
            .connections
            .capture()
            .unwrap()
            .preference()
            .is_none()
    );
    let credentials = fixture.credentials.capture().unwrap();
    assert_eq!(credentials.revision(), &before);
    assert!(credentials.resolve(&provider(), &account()).is_some());
    assert!(fixture.journal.capture().unwrap().is_none());
}

// remove의 모든 durable effect 직후 process가 끊겨도 intent-only는 abandon하고 public commit
// 이후는 새 production session recovery가 credential removal과 journal cleanup까지 수렴합니다.
#[test]
fn every_remove_effect_cut_has_exact_durable_state_and_retry_convergence() {
    for step in [
        DisconnectStep::JournalPublished,
        DisconnectStep::PublicCommitted,
        DisconnectStep::JournalAdvanced(ConnectionOperationPhase::PublicCommitted),
        DisconnectStep::CredentialRemoved,
        DisconnectStep::JournalAdvanced(ConnectionOperationPhase::CredentialRemoved),
        DisconnectStep::JournalAdvanced(ConnectionOperationPhase::Complete),
        DisconnectStep::JournalCleared,
    ] {
        let fixture = Fixture::new(&format!("disconnect-remove-cut-{step:?}"));
        seed_pair(&fixture);
        let repositories = repositories(&fixture);
        let mut session = repositories.acquire().unwrap();
        let prepared = prepared(
            &mut session,
            &fixture,
            ExternalDisconnectCredentialAction::Remove,
        );

        assert!(matches!(
            session.commit_external_disconnect_until(prepared, step),
            Err(ConnectionOperationExecutionError::InjectedInterruption)
        ));
        drop(session);

        let intent_only = step == DisconnectStep::JournalPublished;
        assert_eq!(
            fixture.connections.capture().unwrap().preference().cloned(),
            intent_only.then(stored_target)
        );
        let credential_present = fixture
            .credentials
            .capture()
            .unwrap()
            .resolve(&provider(), &account())
            .is_some();
        assert_eq!(
            credential_present,
            intent_only
                || matches!(
                    step,
                    DisconnectStep::PublicCommitted
                        | DisconnectStep::JournalAdvanced(
                            ConnectionOperationPhase::PublicCommitted
                        )
                )
        );

        let outcome = repositories
            .acquire()
            .unwrap()
            .recover_pending_operation()
            .unwrap();
        if intent_only {
            assert!(matches!(
                outcome,
                ConnectionOperationExecutionOutcome::Abandoned { .. }
            ));
            assert_eq!(
                fixture.connections.capture().unwrap().preference(),
                Some(&stored_target())
            );
            assert!(
                fixture
                    .credentials
                    .capture()
                    .unwrap()
                    .resolve(&provider(), &account())
                    .is_some()
            );
        } else {
            assert!(matches!(
                outcome,
                ConnectionOperationExecutionOutcome::Completed { .. }
                    | ConnectionOperationExecutionOutcome::NoPendingOperation
            ));
            assert!(
                fixture
                    .connections
                    .capture()
                    .unwrap()
                    .preference()
                    .is_none()
            );
            assert!(
                fixture
                    .credentials
                    .capture()
                    .unwrap()
                    .resolve(&provider(), &account())
                    .is_none()
            );
        }
        assert!(fixture.journal.capture().unwrap().is_none());
    }
}

// preserve의 모든 durable effect 절단점도 intent-only는 원상 유지하고 public 이후는 secret
// revision을 바꾸지 않은 채 recovery가 complete/clear로 수렴합니다.
#[test]
fn every_preserve_effect_cut_keeps_credential_and_retries() {
    for step in [
        DisconnectStep::JournalPublished,
        DisconnectStep::PublicCommitted,
        DisconnectStep::JournalAdvanced(ConnectionOperationPhase::PublicCommitted),
        DisconnectStep::JournalAdvanced(ConnectionOperationPhase::Complete),
        DisconnectStep::JournalCleared,
    ] {
        let fixture = Fixture::new(&format!("disconnect-preserve-cut-{step:?}"));
        seed_pair(&fixture);
        let credential_revision = fixture.credentials.capture().unwrap().revision().clone();
        let repositories = repositories(&fixture);
        let mut session = repositories.acquire().unwrap();
        let prepared = prepared(
            &mut session,
            &fixture,
            ExternalDisconnectCredentialAction::Preserve,
        );

        assert!(matches!(
            session.commit_external_disconnect_until(prepared, step),
            Err(ConnectionOperationExecutionError::InjectedInterruption)
        ));
        drop(session);
        repositories
            .acquire()
            .unwrap()
            .recover_pending_operation()
            .unwrap();

        let credentials = fixture.credentials.capture().unwrap();
        assert_eq!(credentials.revision(), &credential_revision);
        assert!(credentials.resolve(&provider(), &account()).is_some());
        assert!(fixture.journal.capture().unwrap().is_none());
    }
}

// credential winner가 prepare 뒤 바뀌면 public removal까지만 durable하고 private winner를
// 덮어쓰지 않으며, 남은 journal은 다음 recovery에서도 typed conflict로 보존됩니다.
#[test]
fn credential_removal_conflict_preserves_winner_and_recoverable_journal() {
    let fixture = Fixture::new("disconnect-credential-conflict");
    seed_pair(&fixture);
    let repositories = repositories(&fixture);
    let mut session = repositories.acquire().unwrap();
    let prepared = prepared(
        &mut session,
        &fixture,
        ExternalDisconnectCredentialAction::Remove,
    );
    let winner = fixture
        .credentials
        .prepare_set(&provider(), &account())
        .unwrap();
    fixture
        .credentials
        .commit(
            &winner,
            Some(&ApiCredential::new("concurrent-winner").unwrap()),
        )
        .unwrap();

    let error = session
        .commit_external_disconnect(prepared)
        .expect_err("the stale credential removal must conflict");

    assert!(error.to_string().contains("credential repository"));
    assert!(
        fixture
            .connections
            .capture()
            .unwrap()
            .preference()
            .is_none()
    );
    assert_eq!(
        fixture
            .credentials
            .capture()
            .unwrap()
            .resolve(&provider(), &account())
            .unwrap()
            .expose_secret(),
        "concurrent-winner"
    );
    assert!(fixture.journal.capture().unwrap().is_some());
    drop(session);
    assert!(
        repositories
            .acquire()
            .unwrap()
            .recover_pending_operation()
            .is_err()
    );
    assert!(fixture.journal.capture().unwrap().is_some());
}

// 마지막 dependent binding이 없어 remove가 필요하지만 credential이 이미 없으면 preserve나
// 새 revision을 꾸며내지 않고 mutation 전 fail-closed 합니다.
#[test]
fn required_removal_rejects_an_absent_credential_before_intent() {
    let fixture = Fixture::new("disconnect-absent-credential");
    seed_stored(&fixture);
    let repositories = repositories(&fixture);
    let mut session = repositories.acquire().unwrap();

    let error = session
        .prepare_external_disconnect(
            fixture.connections.capture().unwrap().revision(),
            &selection(),
            ExternalDisconnectCredentialAction::Remove,
        )
        .expect_err("an absent credential cannot become a remove mutation");

    assert!(
        error
            .to_string()
            .contains("no stored credential required by the disconnect plan")
    );
    assert!(fixture.journal.capture().unwrap().is_none());
    assert_eq!(
        fixture.connections.capture().unwrap().preference(),
        Some(&stored_target())
    );
}

// 다른 dependent binding 때문에 preserve가 필요해도 required credential 자체가 없으면
// remove 전용 문구를 내지 않고 같은 action-neutral 진단으로 mutation 전에 중단합니다.
#[test]
fn required_preservation_rejects_an_absent_credential_before_intent() {
    let fixture = Fixture::new("disconnect-absent-preserved-credential");
    seed_stored(&fixture);
    let repositories = repositories(&fixture);
    let mut session = repositories.acquire().unwrap();

    let error = session
        .prepare_external_disconnect(
            fixture.connections.capture().unwrap().revision(),
            &selection(),
            ExternalDisconnectCredentialAction::Preserve,
        )
        .expect_err("an absent credential cannot satisfy a preserve plan");

    assert!(
        error
            .to_string()
            .contains("no stored credential required by the disconnect plan")
    );
    assert!(fixture.journal.capture().unwrap().is_none());
    assert_eq!(
        fixture.connections.capture().unwrap().preference(),
        Some(&stored_target())
    );
    assert!(
        fixture
            .credentials
            .capture()
            .unwrap()
            .resolve(&provider(), &account())
            .is_none()
    );
}

// CLI preview에 사용한 public revision 뒤 다른 writer가 connections.yaml을 바꾸면 core가 새
// 상태에서 같은 target을 다시 계획하지 않고 intent 전에 exact public conflict로 중단합니다.
#[test]
fn public_change_after_preview_is_rejected_before_disconnect_preparation() {
    let fixture = Fixture::new("disconnect-public-change");
    seed_pair(&fixture);
    let expected = fixture.connections.capture().unwrap().revision().clone();
    let winner = fixture
        .connections
        .capture()
        .unwrap()
        .prepare_preference(Some(StartupTarget::host_codex()))
        .unwrap()
        .unwrap();
    fixture.connections.commit(&winner).unwrap();
    let repositories = repositories(&fixture);
    let mut session = repositories.acquire().unwrap();

    let error = session
        .prepare_external_disconnect(
            &expected,
            &selection(),
            ExternalDisconnectCredentialAction::Remove,
        )
        .expect_err("a stale preview revision must not prepare a different plan");

    assert!(error.to_string().contains("connection revision conflict"));
    assert_eq!(
        fixture.connections.capture().unwrap().preference(),
        Some(&StartupTarget::host_codex())
    );
    assert!(fixture.journal.capture().unwrap().is_none());
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
    action: ExternalDisconnectCredentialAction,
) -> crate::model_service::PreparedExternalDisconnect {
    session
        .prepare_external_disconnect(
            fixture.connections.capture().unwrap().revision(),
            &selection(),
            action,
        )
        .unwrap()
}

fn seed_pair(fixture: &Fixture) {
    let mutation = fixture
        .credentials
        .prepare_set(&provider(), &account())
        .unwrap();
    fixture
        .credentials
        .commit(&mutation, Some(&candidate()))
        .unwrap();
    seed_stored(fixture);
}

fn seed_stored(fixture: &Fixture) {
    let complete = crate::model_service::CompleteModelBinding::from_durable_json(
        r#"{"provider":"qwencloud","account":"default","model":"alpha","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1"}"#,
    )
    .unwrap();
    let account = ConnectionAccount::new(provider(), account(), None, None).unwrap();
    let binding = StoredModelBinding::new(complete, None).unwrap();
    let mutation = fixture
        .connections
        .capture()
        .unwrap()
        .prepare_model_upsert(account, binding)
        .unwrap()
        .unwrap();
    fixture.connections.commit(&mutation).unwrap();
}

fn selection() -> ModelSelection {
    ModelSelection::new(provider(), account(), ModelId::new("alpha").unwrap())
}

fn stored_target() -> StartupTarget {
    StartupTarget::Model(selection())
}
