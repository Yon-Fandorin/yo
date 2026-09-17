use super::*;
use crate::model_service::ApiCredential;

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
            Some(&StartupTarget::host_codex())
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
