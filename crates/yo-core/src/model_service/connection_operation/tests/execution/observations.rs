use super::*;

// directory identity를 lock 전에 잡고 lock 직후 다시 검사하므로, 두 시점 사이에 pathname이
// 새 directory나 symlink로 바뀌면 acquire 자체가 실패하고 원본과 대체 journal을 보존합니다.
#[test]
fn acquisition_rejects_directory_swap_between_lock_and_identity_recheck() {
    for symlink_retarget in [false, true] {
        let fixture = Fixture::new(if symlink_retarget {
            "execution-acquire-symlink"
        } else {
            "execution-acquire-swap"
        });
        let entry = fixture.connect_entry();
        publish_at_phase(&fixture, &entry, ConnectionOperationPhase::Intent);
        let original_directory = fixture.connections.path().parent().unwrap().to_owned();
        let displaced_directory = original_directory
            .parent()
            .unwrap()
            .join("acquire-displaced-state");
        let original_journal = fs::read(fixture.journal.path()).unwrap();
        let replacement_journal = original_directory.join("connection-operation.yaml");
        let repositories = repositories(&fixture);

        let result = repositories.acquire_after_lock(|| {
            fs::rename(&original_directory, &displaced_directory).unwrap();
            if symlink_retarget {
                let unrelated = original_directory
                    .parent()
                    .unwrap()
                    .join("acquire-unrelated-state");
                fs::create_dir(&unrelated).unwrap();
                fs::write(unrelated.join("connection-operation.yaml"), b"unrelated\n").unwrap();
                symlink(&unrelated, &original_directory).unwrap();
            } else {
                fs::create_dir(&original_directory).unwrap();
                fs::write(&replacement_journal, b"replacement\n").unwrap();
            }
        });

        assert!(matches!(
            result,
            Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
                repository: ConnectionOperationRepositoryKind::Public,
                ..
            })
        ));
        assert_eq!(
            fs::read(displaced_directory.join("connection-operation.yaml")).unwrap(),
            original_journal
        );
        let replacement_bytes = fs::read(replacement_journal).unwrap();
        assert_eq!(
            replacement_bytes,
            if symlink_retarget {
                b"unrelated\n".as_slice()
            } else {
                b"replacement\n".as_slice()
            }
        );
    }
}

// connect recovery를 실제 실행하다 각 phase/public-CAS/clear 직후 중단하면 journal phase와
// 두 revision이 정확한 cut state를 보이고, 새 session은 secret 없이 같은 계획으로 수렴합니다.
#[test]
fn connect_execution_interruptions_preserve_exact_order_and_retry_converges() {
    for step in [
        RecoveryStep::JournalAdvanced(ConnectionOperationPhase::CredentialCommitted),
        RecoveryStep::PublicCommitted,
        RecoveryStep::JournalAdvanced(ConnectionOperationPhase::PublicCommitted),
        RecoveryStep::JournalAdvanced(ConnectionOperationPhase::Complete),
        RecoveryStep::JournalCleared,
    ] {
        let fixture = Fixture::new(&format!("execution-connect-failpoint-{step:?}"));
        let entry = fixture.connect_entry();
        publish_at_phase(&fixture, &entry, ConnectionOperationPhase::Intent);
        fixture
            .credentials
            .commit(entry.credential_mutation().unwrap(), Some(&candidate()))
            .unwrap();

        assert!(matches!(
            recover_until(&fixture, step),
            Err(ConnectionOperationExecutionError::InjectedInterruption)
        ));
        assert_eq!(
            fixture.credentials.capture().unwrap().revision(),
            entry.credential_mutation().unwrap().planned_revision()
        );
        let public_is_planned = !matches!(
            step,
            RecoveryStep::JournalAdvanced(ConnectionOperationPhase::CredentialCommitted)
        );
        assert_eq!(
            fixture.connections.capture().unwrap().revision(),
            if public_is_planned {
                entry.connection_mutation().planned_revision()
            } else {
                entry.connection_mutation().expected_revision()
            }
        );
        assert_journal_cut(&fixture, step);

        assert!(matches!(
            recover(&fixture).unwrap(),
            ConnectionOperationExecutionOutcome::Completed { .. }
                | ConnectionOperationExecutionOutcome::NoPendingOperation
        ));
        assert!(fixture.journal.capture().unwrap().is_none());
        assert_eq!(
            fixture.connections.capture().unwrap().revision(),
            entry.connection_mutation().planned_revision()
        );
    }
}

// disconnect remove recovery를 phase와 credential-CAS 경계마다 끊어 credential 삭제가
// public_committed phase 뒤에만 나타나고 모든 durable cut이 재시도로 수렴함을 검증합니다.
#[test]
fn disconnect_remove_execution_interruptions_preserve_exact_order_and_retry_converges() {
    for step in [
        RecoveryStep::JournalAdvanced(ConnectionOperationPhase::PublicCommitted),
        RecoveryStep::CredentialRemoved,
        RecoveryStep::JournalAdvanced(ConnectionOperationPhase::CredentialRemoved),
        RecoveryStep::JournalAdvanced(ConnectionOperationPhase::Complete),
        RecoveryStep::JournalCleared,
    ] {
        let fixture = Fixture::new(&format!("execution-remove-failpoint-{step:?}"));
        let entry = fixture.seed_disconnect_remove();
        publish_at_phase(&fixture, &entry, ConnectionOperationPhase::Intent);
        fixture
            .connections
            .commit(entry.connection_mutation())
            .unwrap();

        assert!(matches!(
            recover_until(&fixture, step),
            Err(ConnectionOperationExecutionError::InjectedInterruption)
        ));
        assert_eq!(
            fixture.connections.capture().unwrap().revision(),
            entry.connection_mutation().planned_revision()
        );
        let credential_is_removed = !matches!(
            step,
            RecoveryStep::JournalAdvanced(ConnectionOperationPhase::PublicCommitted)
        );
        assert_eq!(
            fixture.credentials.capture().unwrap().revision(),
            if credential_is_removed {
                entry.credential_mutation().unwrap().planned_revision()
            } else {
                entry.credential_mutation().unwrap().expected_revision()
            }
        );
        assert_journal_cut(&fixture, step);

        assert!(matches!(
            recover(&fixture).unwrap(),
            ConnectionOperationExecutionOutcome::Completed { .. }
                | ConnectionOperationExecutionOutcome::NoPendingOperation
        ));
        assert!(fixture.journal.capture().unwrap().is_none());
        assert_eq!(
            fixture.credentials.capture().unwrap().revision(),
            entry.credential_mutation().unwrap().planned_revision()
        );
    }
}

// abandon과 preserve도 실제 clear/phase 경계에서 중단해 abandon은 두 저장소를 expected로
// 남기고, preserve는 credential revision을 끝까지 바꾸지 않은 채 재시도 수렴함을 검증합니다.
#[test]
fn abandon_and_preserve_execution_interruptions_converge_without_credential_mutation() {
    let abandon_fixture = Fixture::new("execution-abandon-failpoint");
    let abandon = abandon_fixture.connect_entry();
    publish_at_phase(&abandon_fixture, &abandon, ConnectionOperationPhase::Intent);
    assert!(matches!(
        recover_until(&abandon_fixture, RecoveryStep::JournalAbandoned),
        Err(ConnectionOperationExecutionError::InjectedInterruption)
    ));
    assert!(abandon_fixture.journal.capture().unwrap().is_none());
    assert_eq!(
        abandon_fixture.connections.capture().unwrap().revision(),
        abandon.connection_mutation().expected_revision()
    );
    assert_eq!(
        abandon_fixture.credentials.capture().unwrap().revision(),
        abandon.expected_credential_revision()
    );
    assert_eq!(
        recover(&abandon_fixture).unwrap(),
        ConnectionOperationExecutionOutcome::NoPendingOperation
    );

    for step in [
        RecoveryStep::JournalAdvanced(ConnectionOperationPhase::PublicCommitted),
        RecoveryStep::JournalAdvanced(ConnectionOperationPhase::Complete),
        RecoveryStep::JournalCleared,
    ] {
        let fixture = Fixture::new(&format!("execution-preserve-failpoint-{step:?}"));
        let entry = fixture.seed_disconnect_preserve();
        publish_at_phase(&fixture, &entry, ConnectionOperationPhase::Intent);
        fixture
            .connections
            .commit(entry.connection_mutation())
            .unwrap();

        assert!(matches!(
            recover_until(&fixture, step),
            Err(ConnectionOperationExecutionError::InjectedInterruption)
        ));
        assert_eq!(
            fixture.credentials.capture().unwrap().revision(),
            entry.expected_credential_revision()
        );
        assert_journal_cut(&fixture, step);
        assert!(matches!(
            recover(&fixture).unwrap(),
            ConnectionOperationExecutionOutcome::Completed { .. }
                | ConnectionOperationExecutionOutcome::NoPendingOperation
        ));
        assert_eq!(
            fixture.credentials.capture().unwrap().revision(),
            entry.expected_credential_revision()
        );
    }
}

pub(super) fn recover_until(
    fixture: &Fixture,
    step: RecoveryStep,
) -> Result<ConnectionOperationExecutionOutcome, ConnectionOperationExecutionError> {
    repositories(fixture)
        .acquire()
        .unwrap()
        .recover_pending_operation_until(step)
}

pub(super) fn assert_journal_cut(fixture: &Fixture, step: RecoveryStep) {
    let expected = match step {
        RecoveryStep::JournalAdvanced(phase) => Some(phase),
        RecoveryStep::PublicCommitted => Some(ConnectionOperationPhase::CredentialCommitted),
        RecoveryStep::CredentialRemoved => Some(ConnectionOperationPhase::PublicCommitted),
        RecoveryStep::JournalAbandoned | RecoveryStep::JournalCleared => None,
    };
    assert_eq!(
        fixture
            .journal
            .capture()
            .unwrap()
            .map(|entry| entry.phase()),
        expected
    );
}
