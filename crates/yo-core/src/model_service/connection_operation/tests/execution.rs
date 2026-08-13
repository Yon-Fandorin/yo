use std::fs;

use super::support::{CANDIDATE_SECRET, Fixture, account, candidate, provider};
use crate::model_service::{
    AccountId, ApiCredential, ConnectionCredentialAction, ConnectionOperationError,
    ConnectionOperationExecutionError, ConnectionOperationExecutionOutcome,
    ConnectionOperationPhase, ConnectionOperationRepositoryKind,
    LocalConnectionOperationRepositories, ProviderId, StartupTarget,
};

// 저널이 없으면 세 저장소를 만들거나 변경하지 않고 NoPendingOperation을 반환하며, 같은
// session이 operation lock을 계속 보유해 호출자가 새 계획을 안전하게 이어갈 수 있습니다.
#[test]
fn execution_without_a_journal_is_a_non_mutating_noop() {
    let fixture = Fixture::new("execution-none");
    let repositories = repositories(&fixture);
    let mut session = repositories.acquire().unwrap();

    assert_eq!(
        session.recover_pending_operation().unwrap(),
        ConnectionOperationExecutionOutcome::NoPendingOperation
    );
    assert!(fixture.journal.capture().unwrap().is_none());
    assert!(
        fixture
            .connections
            .capture()
            .unwrap()
            .revision()
            .is_absent()
    );
    assert!(
        fixture
            .credentials
            .capture()
            .unwrap()
            .revision()
            .is_absent()
    );
}

// connect intent만 있고 두 저장소가 expected인 절단점은 secret을 복원하거나 credential
// commit을 시도하지 않고 intent를 제거해 다음 호출이 candidate 검증부터 다시 시작합니다.
#[test]
fn connect_before_first_commit_abandons_without_reusing_a_secret() {
    let fixture = Fixture::new("execution-connect-abandon");
    let entry = fixture.connect_entry();
    publish_at_phase(&fixture, &entry, ConnectionOperationPhase::Intent);

    assert_eq!(
        recover(&fixture).unwrap(),
        ConnectionOperationExecutionOutcome::Abandoned {
            kind: entry.kind(),
            action: ConnectionCredentialAction::Add,
        }
    );
    assert!(fixture.journal.capture().unwrap().is_none());
    assert!(fixture.credentials.capture().unwrap().is_empty());
    assert!(
        fixture
            .connections
            .capture()
            .unwrap()
            .preference()
            .is_none()
    );
}

// replace credential이 이미 candidate로 durable하지만 public CAS 전인 모든 합법 journal
// phase에서 executor는 저장된 이전 secret을 재사용하지 않고 exact public bytes만 완결합니다.
#[test]
fn connect_credential_first_cut_points_complete_public_without_secret_reuse() {
    for phase in [
        ConnectionOperationPhase::Intent,
        ConnectionOperationPhase::CredentialCommitted,
    ] {
        let fixture = Fixture::new(&format!("execution-connect-credential-{phase:?}"));
        seed_credential(&fixture, "old-secret");
        let entry = fixture.connect_entry();
        publish_at_phase(&fixture, &entry, phase);
        fixture
            .credentials
            .commit(entry.credential_mutation().unwrap(), Some(&candidate()))
            .unwrap();

        assert_eq!(
            recover(&fixture).unwrap(),
            ConnectionOperationExecutionOutcome::Completed {
                kind: entry.kind(),
                action: ConnectionCredentialAction::Replace,
                recovered_from: phase,
            }
        );
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
}

// connect의 두 저장소가 이미 exact planned인 뒤 저널만 뒤처진 모든 phase에서는 planned
// 상태를 유지하면서 phase를 durable하게 따라잡아 complete journal을 지웁니다.
#[test]
fn connect_repository_ahead_cut_points_catch_up_and_clear() {
    for phase in [
        ConnectionOperationPhase::Intent,
        ConnectionOperationPhase::CredentialCommitted,
        ConnectionOperationPhase::PublicCommitted,
        ConnectionOperationPhase::Complete,
    ] {
        let fixture = Fixture::new(&format!("execution-connect-complete-{phase:?}"));
        let entry = fixture.connect_entry();
        publish_at_phase(&fixture, &entry, phase);
        fixture
            .credentials
            .commit(entry.credential_mutation().unwrap(), Some(&candidate()))
            .unwrap();
        fixture
            .connections
            .commit(entry.connection_mutation())
            .unwrap();

        assert!(matches!(
            recover(&fixture).unwrap(),
            ConnectionOperationExecutionOutcome::Completed {
                recovered_from,
                ..
            } if recovered_from == phase
        ));
        assert!(fixture.journal.capture().unwrap().is_none());
    }
}

// disconnect remove에서 public만 planned인 두 합법 phase는 공개 상태를 먼저 인정한 뒤 exact
// remove를 None candidate로 적용하며, credential까지 planned인 나머지 phase도 따라잡습니다.
#[test]
fn disconnect_remove_cut_points_remove_only_after_public_commit() {
    for phase in [
        ConnectionOperationPhase::Intent,
        ConnectionOperationPhase::PublicCommitted,
    ] {
        let fixture = Fixture::new(&format!("execution-remove-public-{phase:?}"));
        let entry = fixture.seed_disconnect_remove();
        publish_at_phase(&fixture, &entry, phase);
        fixture
            .connections
            .commit(entry.connection_mutation())
            .unwrap();

        assert!(matches!(
            recover(&fixture).unwrap(),
            ConnectionOperationExecutionOutcome::Completed {
                action: ConnectionCredentialAction::Remove,
                recovered_from,
                ..
            } if recovered_from == phase
        ));
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

    for phase in [
        ConnectionOperationPhase::Intent,
        ConnectionOperationPhase::PublicCommitted,
        ConnectionOperationPhase::CredentialRemoved,
        ConnectionOperationPhase::Complete,
    ] {
        let fixture = Fixture::new(&format!("execution-remove-complete-{phase:?}"));
        let entry = fixture.seed_disconnect_remove();
        publish_at_phase(&fixture, &entry, phase);
        fixture
            .connections
            .commit(entry.connection_mutation())
            .unwrap();
        fixture
            .credentials
            .commit(entry.credential_mutation().unwrap(), None)
            .unwrap();

        assert!(matches!(
            recover(&fixture).unwrap(),
            ConnectionOperationExecutionOutcome::Completed {
                recovered_from,
                ..
            } if recovered_from == phase
        ));
        assert!(fixture.journal.capture().unwrap().is_none());
    }
}

// disconnect의 remove와 preserve intent가 아직 expected/expected이면 어느 쪽도 공개 상태나
// credential을 바꾸지 않고 intent만 abandon해 새 호출이 현재 binding 집합을 다시 계산합니다.
#[test]
fn disconnect_before_public_commit_abandons_both_credential_actions() {
    let remove_fixture = Fixture::new("execution-remove-abandon");
    let remove = remove_fixture.seed_disconnect_remove();
    publish_at_phase(&remove_fixture, &remove, ConnectionOperationPhase::Intent);

    assert_eq!(
        recover(&remove_fixture).unwrap(),
        ConnectionOperationExecutionOutcome::Abandoned {
            kind: remove.kind(),
            action: ConnectionCredentialAction::Remove,
        }
    );
    assert!(
        remove_fixture
            .credentials
            .capture()
            .unwrap()
            .resolve(&provider(), &account())
            .is_some()
    );
    assert_eq!(
        remove_fixture.connections.capture().unwrap().preference(),
        Some(&StartupTarget::HostCodex)
    );

    let preserve_fixture = Fixture::new("execution-preserve-abandon");
    let preserve = preserve_fixture.seed_disconnect_preserve();
    publish_at_phase(
        &preserve_fixture,
        &preserve,
        ConnectionOperationPhase::Intent,
    );

    assert_eq!(
        recover(&preserve_fixture).unwrap(),
        ConnectionOperationExecutionOutcome::Abandoned {
            kind: preserve.kind(),
            action: ConnectionCredentialAction::Preserve,
        }
    );
    assert!(
        preserve_fixture
            .credentials
            .capture()
            .unwrap()
            .resolve(&provider(), &account())
            .is_some()
    );
    assert_eq!(
        preserve_fixture.connections.capture().unwrap().preference(),
        Some(&StartupTarget::HostCodex)
    );
}

// disconnect preserve에서 public CAS 뒤의 모든 phase는 credential 파일을 건드리지 않고
// journal만 완결하며, 동일 pair의 secret이 그대로 남는 것을 직접 비교합니다.
#[test]
fn disconnect_preserve_cut_points_leave_credential_unchanged() {
    for phase in [
        ConnectionOperationPhase::Intent,
        ConnectionOperationPhase::PublicCommitted,
        ConnectionOperationPhase::Complete,
    ] {
        let fixture = Fixture::new(&format!("execution-preserve-{phase:?}"));
        let entry = fixture.seed_disconnect_preserve();
        publish_at_phase(&fixture, &entry, phase);
        fixture
            .connections
            .commit(entry.connection_mutation())
            .unwrap();

        assert!(matches!(
            recover(&fixture).unwrap(),
            ConnectionOperationExecutionOutcome::Completed {
                action: ConnectionCredentialAction::Preserve,
                recovered_from,
                ..
            } if recovered_from == phase
        ));
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
        assert!(fixture.journal.capture().unwrap().is_none());
    }
}

// 저널 phase가 저장소보다 앞서거나 다른 credential revision이 이긴 경우 실행기는 conflict를
// 반환하고 현재 저널 bytes를 보존하며 private revision과 secret을 진단에 노출하지 않습니다.
#[test]
fn phase_ahead_and_different_state_conflicts_preserve_private_journal() {
    let phase_ahead = Fixture::new("execution-phase-ahead");
    let entry = phase_ahead.connect_entry();
    publish_at_phase(
        &phase_ahead,
        &entry,
        ConnectionOperationPhase::PublicCommitted,
    );
    let before = fs::read(phase_ahead.journal.path()).unwrap();
    let private_revision = entry
        .credential_mutation()
        .unwrap()
        .planned_revision()
        .operation_journal_token()
        .to_owned();

    let error = recover(&phase_ahead).unwrap_err();
    assert!(matches!(
        error,
        ConnectionOperationExecutionError::Journal {
            source: ConnectionOperationError::RecoveryConflict { .. },
            ..
        }
    ));
    assert_eq!(fs::read(phase_ahead.journal.path()).unwrap(), before);
    let diagnostic = format!("{error:?} {error}");
    assert!(!diagnostic.contains(&private_revision));
    assert!(!diagnostic.contains(CANDIDATE_SECRET));

    let different = Fixture::new("execution-different-state");
    let entry = different.connect_entry();
    publish_at_phase(&different, &entry, ConnectionOperationPhase::Intent);
    seed_other_credential(&different);
    let before = fs::read(different.journal.path()).unwrap();

    assert!(matches!(
        recover(&different),
        Err(ConnectionOperationExecutionError::Journal {
            source: ConnectionOperationError::RecoveryConflict { .. },
            ..
        })
    ));
    assert_eq!(fs::read(different.journal.path()).unwrap(), before);
}

// 서로 다른 directory나 closed filename을 조합한 bundle은 lock 획득과 journal capture 전에
// 거절되어 이미 존재하는 pending journal의 identity와 phase bytes를 전혀 변경하지 않습니다.
#[test]
fn incompatible_repository_layout_is_rejected_before_journal_mutation() {
    let fixture = Fixture::new("execution-layout");
    let entry = fixture.connect_entry();
    publish_at_phase(&fixture, &entry, ConnectionOperationPhase::Intent);
    let before = fs::read(fixture.journal.path()).unwrap();
    let other_directory = fixture.credentials.path().parent().unwrap().join("other");

    assert!(matches!(
        LocalConnectionOperationRepositories::from_paths(
            fixture.connections.path(),
            other_directory.join("credentials.yaml"),
            fixture.journal.path(),
        ),
        Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
            repository: ConnectionOperationRepositoryKind::Credential,
            ..
        })
    ));
    assert!(matches!(
        LocalConnectionOperationRepositories::from_paths(
            fixture.connections.path(),
            fixture.credentials.path(),
            fixture.journal.path().with_file_name("pending.yaml"),
        ),
        Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
            repository: ConnectionOperationRepositoryKind::Journal,
            ..
        })
    ));
    assert_eq!(fs::read(fixture.journal.path()).unwrap(), before);
}

fn repositories(fixture: &Fixture) -> LocalConnectionOperationRepositories {
    LocalConnectionOperationRepositories::from_paths(
        fixture.connections.path(),
        fixture.credentials.path(),
        fixture.journal.path(),
    )
    .unwrap()
}

fn recover(
    fixture: &Fixture,
) -> Result<ConnectionOperationExecutionOutcome, ConnectionOperationExecutionError> {
    repositories(fixture)
        .acquire()
        .unwrap()
        .recover_pending_operation()
}

fn publish_at_phase(
    fixture: &Fixture,
    entry: &super::super::ConnectionOperationJournalEntry,
    target: ConnectionOperationPhase,
) {
    let mut guard = fixture.operation_guard();
    let mut current = entry.clone();
    fixture
        .journal
        .publish_intent(&mut guard, &current)
        .unwrap();
    while current.phase() != target {
        let next = current.next_phase().unwrap();
        current = fixture.journal.advance(&mut guard, &current, next).unwrap();
    }
}

fn seed_credential(fixture: &Fixture, value: &str) {
    let mutation = fixture
        .credentials
        .prepare_set(&provider(), &account())
        .unwrap();
    fixture
        .credentials
        .commit(&mutation, Some(&ApiCredential::new(value).unwrap()))
        .unwrap();
}

fn seed_other_credential(fixture: &Fixture) {
    let provider = ProviderId::new("openrouter").unwrap();
    let account = AccountId::new("other").unwrap();
    let mutation = fixture
        .credentials
        .prepare_set(&provider, &account)
        .unwrap();
    fixture
        .credentials
        .commit(
            &mutation,
            Some(&ApiCredential::new("other-secret").unwrap()),
        )
        .unwrap();
}
