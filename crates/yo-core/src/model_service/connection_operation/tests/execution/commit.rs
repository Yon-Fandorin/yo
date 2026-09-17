use super::{observations::seed_other_credential, *};

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
        Some(&StartupTarget::host_codex())
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
        Some(&StartupTarget::host_codex())
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

// acquire 뒤 state directory pathname을 새 inode나 symlink로 바꾸면 retained lock이 예전
// directory에 남아 있어도 executor는 다음 capture 전에 실패하고 양쪽 journal을 보존합니다.
#[test]
fn acquired_directory_identity_rejects_swap_and_symlink_retarget_before_mutation() {
    for symlink_retarget in [false, true] {
        let fixture = Fixture::new(if symlink_retarget {
            "execution-symlink-retarget"
        } else {
            "execution-directory-swap"
        });
        let entry = fixture.connect_entry();
        publish_at_phase(&fixture, &entry, ConnectionOperationPhase::Intent);
        let original_directory = fixture.connections.path().parent().unwrap().to_owned();
        let displaced_directory = original_directory.parent().unwrap().join("displaced-state");
        let original_journal = fs::read(fixture.journal.path()).unwrap();
        let repositories = repositories(&fixture);
        let mut session = repositories.acquire().unwrap();

        fs::rename(&original_directory, &displaced_directory).unwrap();
        let replacement_journal = if symlink_retarget {
            let unrelated = original_directory.parent().unwrap().join("unrelated-state");
            fs::create_dir(&unrelated).unwrap();
            let replacement = unrelated.join("connection-operation.yaml");
            fs::write(&replacement, b"unrelated\n").unwrap();
            symlink(&unrelated, &original_directory).unwrap();
            replacement
        } else {
            fs::create_dir(&original_directory).unwrap();
            let replacement = original_directory.join("connection-operation.yaml");
            fs::write(&replacement, b"replacement\n").unwrap();
            replacement
        };
        let replacement_before = fs::read(&replacement_journal).unwrap();

        assert!(matches!(
            session.recover_pending_operation(),
            Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
                repository: ConnectionOperationRepositoryKind::Public,
                ..
            })
        ));
        assert_eq!(
            fs::read(displaced_directory.join("connection-operation.yaml")).unwrap(),
            original_journal
        );
        assert_eq!(fs::read(replacement_journal).unwrap(), replacement_before);
    }
}

pub(super) fn seed_credential(fixture: &Fixture, value: &str) {
    let mutation = fixture
        .credentials
        .prepare_set(&provider(), &account())
        .unwrap();
    fixture
        .credentials
        .commit(&mutation, Some(&ApiCredential::new(value).unwrap()))
        .unwrap();
}
