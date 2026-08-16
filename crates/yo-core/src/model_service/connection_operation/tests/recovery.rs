use std::{fs, os::unix::fs::PermissionsExt};

use super::{
    super::{
        ConnectionOperationError, ConnectionOperationPhase, ConnectionOperationRecovery,
        plan_connection_recovery,
    },
    support::{Fixture, candidate},
};

// connect intent의 두 repository가 모두 expected이면 아직 secret commit이 없으므로 intent를
// abandon하고 다음 invocation이 candidate secret 검증부터 다시 시작해야 함을 검증합니다.
#[test]
fn connect_expected_expected_abandons_uncommitted_intent() {
    let fixture = Fixture::new("connect-abandon");
    let entry = fixture.connect_entry();

    assert_eq!(
        recover(&fixture, &entry).unwrap(),
        ConnectionOperationRecovery::Abandon
    );
}

// credential만 planned인 connect는 secret을 복원하지 않고 journal의 exact public mutation만
// commit하도록 판정하며 public까지 planned가 되면 lagging intent에서도 complete가 됩니다.
#[test]
fn connect_recovers_credential_first_then_completes_exact_public_state() {
    let fixture = Fixture::new("connect-forward");
    let entry = fixture.connect_entry();
    fixture
        .credentials
        .commit(entry.credential_mutation().unwrap(), Some(&candidate()))
        .unwrap();

    assert_eq!(
        recover(&fixture, &entry).unwrap(),
        ConnectionOperationRecovery::CommitPublic
    );
    fixture
        .connections
        .commit(entry.connection_mutation())
        .unwrap();
    assert_eq!(
        recover(&fixture, &entry).unwrap(),
        ConnectionOperationRecovery::Complete
    );
}

// journal phase가 public_committed를 주장하지만 실제 repository가 둘 다 expected이면 단순
// abandon하지 않고 Conflict로 실패해 durable phase claim을 사실과 다르게 되감지 않습니다.
#[test]
fn phase_claim_ahead_of_repositories_is_conflict() {
    let fixture = Fixture::new("phase-ahead");
    let intent = fixture.connect_entry();
    let credential_committed = intent
        .with_phase(ConnectionOperationPhase::CredentialCommitted)
        .unwrap();
    let public_committed = credential_committed
        .with_phase(ConnectionOperationPhase::PublicCommitted)
        .unwrap();

    assert!(matches!(
        recover(&fixture, &public_committed),
        Err(ConnectionOperationError::RecoveryConflict { .. })
    ));
}

// disconnect remove는 expected/expected에서 abandon하고 public CAS가 planned가 된 뒤에만
// exact credential removal을 허용하며 두 planned receipt가 관찰되면 complete가 됩니다.
#[test]
fn disconnect_remove_recovers_public_first_then_removes_credential() {
    let fixture = Fixture::new("disconnect-remove");
    let entry = fixture.seed_disconnect_remove();

    assert_eq!(
        recover(&fixture, &entry).unwrap(),
        ConnectionOperationRecovery::Abandon
    );
    fixture
        .connections
        .commit(entry.connection_mutation())
        .unwrap();
    assert_eq!(
        recover(&fixture, &entry).unwrap(),
        ConnectionOperationRecovery::CommitCredentialRemoval
    );
    fixture
        .credentials
        .commit(entry.credential_mutation().unwrap(), None)
        .unwrap();
    assert_eq!(
        recover(&fixture, &entry).unwrap(),
        ConnectionOperationRecovery::Complete
    );
}

// remove credential이 public CAS보다 먼저 planned가 된 unlisted disconnect state는 recovery가
// 소유권을 추정하거나 public을 덮지 않고 Conflict로 중단함을 검증합니다.
#[test]
fn disconnect_credential_first_state_is_conflict() {
    let fixture = Fixture::new("disconnect-wrong-order");
    let entry = fixture.seed_disconnect_remove();
    fixture
        .credentials
        .commit(entry.credential_mutation().unwrap(), None)
        .unwrap();

    assert!(matches!(
        recover(&fixture, &entry),
        Err(ConnectionOperationError::RecoveryConflict { .. })
    ));
}

// preserve disconnect는 credential revision을 바꾸지 않고 public만 exact planned가 되면
// complete하므로 남은 manual/managed binding이 사용하는 secret을 삭제하지 않음을 검증합니다.
#[test]
fn disconnect_preserve_completes_after_public_without_credential_mutation() {
    let fixture = Fixture::new("disconnect-preserve");
    let entry = fixture.seed_disconnect_preserve();
    let credential_revision = fixture.credentials.capture().unwrap().revision().clone();
    fixture
        .connections
        .commit(entry.connection_mutation())
        .unwrap();

    assert_eq!(
        recover(&fixture, &entry).unwrap(),
        ConnectionOperationRecovery::Complete
    );
    assert_eq!(
        fixture.credentials.capture().unwrap().revision(),
        &credential_revision
    );
}

// planned ConnectionRevision 문자열만 복사하고 snapshot bytes를 바꾼 파일은 exact planned
// state가 아니므로 recovery가 Complete로 오인하지 않고 Conflict를 반환함을 검증합니다.
#[test]
fn planned_public_revision_with_different_bytes_is_conflict() {
    let fixture = Fixture::new("public-bytes-mismatch");
    let entry = fixture.connect_entry();
    fixture
        .credentials
        .commit(entry.credential_mutation().unwrap(), Some(&candidate()))
        .unwrap();
    fs::create_dir_all(fixture.connections.path().parent().unwrap()).unwrap();
    fs::write(
        fixture.connections.path(),
        format!(
            "revision: {}\n",
            entry.connection_mutation().planned_revision()
        ),
    )
    .unwrap();
    fs::set_permissions(
        fixture.connections.path(),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();

    assert!(matches!(
        recover(&fixture, &entry),
        Err(ConnectionOperationError::RecoveryConflict { .. })
    ));
}

fn recover(
    fixture: &Fixture,
    entry: &super::super::ConnectionOperationJournalEntry,
) -> Result<ConnectionOperationRecovery, ConnectionOperationError> {
    plan_connection_recovery(
        entry,
        &fixture.credentials.capture().unwrap(),
        &fixture.connections.capture().unwrap(),
    )
}
