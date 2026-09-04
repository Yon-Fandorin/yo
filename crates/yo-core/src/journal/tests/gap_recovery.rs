use super::*;

// 최초 repository read가 잠시 실패해도 그 사이의 semantic records를 버리지 않고
// 다음 이벤트에서 전체 snapshot으로 재시도해 정상 durable 상태로 돌아가야 한다.
#[test]
fn transient_initial_read_failure_retries_with_the_complete_live_snapshot() {
    let session_id = session(1);
    let repository = TransientReadRepository::default();
    repository.fail_next_read.store(true, Ordering::Release);
    let observed = Arc::clone(&repository.state);
    let mut journal = SessionJournal::with_repository(Box::new(repository));

    assert_eq!(
        journal.transcript_reader().durability(),
        JournalDurability::Gap {
            durable_cutoff: DurableCutoff::Unknown,
            cause: super::super::DurabilityGapCause::Storage,
        }
    );
    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );

    assert!(matches!(
        journal.transcript_reader().durability(),
        JournalDurability::Durable { .. }
    ));
    let state = observed.lock().unwrap();
    assert_eq!(state.entries.len(), 1);
    assert_eq!(
        state.entries[0].record().kind(),
        DurableRecordKind::Snapshot
    );
    let commit = decode(state.entries[0].record().payload()).unwrap();
    assert_eq!(commit.records().len(), 3);
    assert!(recover(&[commit]).unwrap().recovery_commit().is_none());
}

// capacity pressure 뒤의 semantic 결과는 live Journal에 남고, 같은 writer에서 공간이
// 돌아오면 다음 commit이 전체 snapshot을 써 durable gap을 닫은 뒤 Durable로 복귀한다.
#[test]
fn capacity_gap_recovers_with_one_complete_live_snapshot() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let repository = SharedRepository::default();
    let pressure = Arc::clone(&repository.pressure);
    let observed = Arc::clone(&repository.state);
    let mut journal = SessionJournal::with_repository(Box::new(repository));

    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );
    pressure.store(true, Ordering::Release);
    journal.append_committed_submission(
        AgentCommand::StartTurn {
            turn,
            input: UserInput::from("inspect"),
        },
        "10000000-0000-4000-8000-000000000011"
            .parse()
            .expect("the test submission fixture is a UUIDv4"),
        &[AgentEvent::TurnStarted { turn }],
    );
    assert!(matches!(
        journal.transcript_reader().durability(),
        JournalDurability::Gap {
            cause: super::super::DurabilityGapCause::Capacity,
            ..
        }
    ));

    pressure.store(false, Ordering::Release);
    journal.append_events(&[AgentEvent::TurnFinished {
        turn,
        outcome: TurnOutcome::Completed,
    }]);

    assert!(matches!(
        journal.transcript_reader().durability(),
        JournalDurability::Durable { .. }
    ));
    let state = observed.lock().unwrap();
    assert_eq!(state.entries.len(), 3);
    assert_eq!(
        state.entries[2].record().kind(),
        DurableRecordKind::Snapshot
    );
}

// size 경계에서 일부 text를 저장한 message가 열린 동안 capacity gap이 생기면 공간이
// 돌아와도 중간 event에서 불완전 snapshot을 시도하지 않는다. 실제 MessageEnded가 생긴
// 뒤에만 전체 snapshot을 써서 일시적인 압력을 integrity gap으로 오판하지 않아야 한다.
#[test]
fn open_message_defers_gap_snapshot_until_its_terminal_seal() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let message = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let repository = SharedRepository::default();
    let pressure = Arc::clone(&repository.pressure);
    let observed = Arc::clone(&repository.state);
    let mut journal = SessionJournal::with_repository(Box::new(repository));

    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );
    journal.append_events(&[AgentEvent::ActivityStarted {
        activity: message,
        kind: ActivityKind::AgentMessage,
    }]);
    pressure.store(true, Ordering::Release);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity: message,
        update: ActivityUpdate::TextDelta("x".repeat(16 * 1024)),
    }]);

    pressure.store(false, Ordering::Release);
    journal.append_events(&[AgentEvent::TurnStarted { turn }]);
    assert!(matches!(
        journal.transcript_reader().durability(),
        JournalDurability::Gap {
            cause: super::super::DurabilityGapCause::Capacity,
            ..
        }
    ));

    journal.append_events(&[AgentEvent::ActivityFinished {
        activity: message,
        outcome: ActivityOutcome::Completed,
    }]);

    assert!(matches!(
        journal.transcript_reader().durability(),
        JournalDurability::Durable { .. }
    ));
    let state = observed.lock().unwrap();
    assert_eq!(
        state.entries.last().unwrap().record().kind(),
        DurableRecordKind::Snapshot
    );
    let commits = state
        .entries
        .iter()
        .map(|entry| decode(entry.record().payload()).unwrap())
        .collect::<Vec<_>>();
    assert!(recover(&commits).unwrap().recovery_commit().is_none());
}

// repository가 complete snapshot을 요구한다는 것은 현재 live writer가 증분 연속성을
// 증명할 수 없다는 뜻이다. 이 경우 열린 message를 임의의 recovery seal과 합치며 계속
// 재시도하지 않고, typed integrity gap을 유지한 채 이후 의미 기록만 memory에 보존한다.
#[test]
fn integrity_gap_stops_automatic_snapshot_retries() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let message = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let state = Arc::new(Mutex::new(RepositoryState::default()));
    let attempts = Arc::new(Mutex::new(0));
    let repository = SnapshotGateRepository {
        state,
        fail_on_append: 5,
        attempts: Arc::clone(&attempts),
    };
    let mut journal = SessionJournal::with_repository(Box::new(repository));

    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );
    journal.append_events(&[AgentEvent::ActivityStarted {
        activity: message,
        kind: ActivityKind::AgentMessage,
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity: message,
        update: ActivityUpdate::TextDelta("x".repeat(16 * 1024)),
    }]);
    journal.append_events(&[AgentEvent::TurnStarted { turn }]);
    assert!(matches!(
        journal.transcript_reader().durability(),
        JournalDurability::Gap {
            cause: super::super::DurabilityGapCause::Integrity,
            ..
        }
    ));

    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity: message,
        update: ActivityUpdate::TextDelta("volatile".to_owned()),
    }]);
    assert_eq!(*attempts.lock().unwrap(), 5);
}
