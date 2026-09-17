use super::*;

// 표시 cap과 물리 검증 cap을 분리해 첫 초과·후반 discovery 손상을 prefix 성공으로 숨기지 않는다.
#[test]
fn historical_catalog_enforces_full_physical_bounds_and_late_discovery_validation() {
    use crate::session_repository::{
        DurableRecord, RecordDiscovery, SessionForkLimits, SessionRepository, read_fork_catalog,
    };
    let parent = fixture_session(82);
    let directory = ForkRepositoryDirectory(env::temp_dir().join(format!(
        "yo-historical-bounds-{}",
        crate::SessionId::new().unwrap()
    )));
    let (initial, _) = fork_two_group_bootstrap();
    let (request, complete, _) = fork_private_turn(5, 1, 1);
    let mut journal = JournalRepository::new(
        LocalSessionRepository::open(&directory.0, 8 * 1024 * 1024).unwrap(),
    );
    for commit in [&initial, &request, &complete] {
        journal.append(parent, commit).unwrap();
    }
    let reader = LocalSessionReader::open(&directory.0).unwrap();
    let bytes = fs::metadata(directory.0.join(format!("{parent}.jsonl")))
        .unwrap()
        .len();
    let exact_limits = SessionForkLimits::try_new(bytes, 3, 1).unwrap();
    let catalog = read_fork_catalog(&reader, parent, exact_limits).unwrap();
    assert!(catalog.truncated());
    assert_eq!(catalog.boundaries().len(), 1);
    assert_eq!(
        catalog.boundaries()[0].journal_cutoff(),
        JournalSequence::new(11)
    );
    assert!(
        catalog
            .selection(0)
            .unwrap()
            .prepare_source(parent, catalog.durability())
            .is_ok()
    );
    for limits in [
        SessionForkLimits::try_new(bytes - 1, 3, 1).unwrap(),
        SessionForkLimits::try_new(bytes, 2, 1).unwrap(),
    ] {
        assert!(read_fork_catalog(&reader, parent, limits).is_err());
    }
    let recovered = recover(&[initial, request, complete]).unwrap();
    let mut repository = journal.into_inner();
    repository
        .append(
            parent,
            DurableRecord::snapshot(encode(&recovered.complete_snapshot()).unwrap())
                .with_journal_cutoff(recovered.journal_cutoff())
                .with_discovery(
                    RecordDiscovery::new(fixture_descriptor(parent)).with_binding_epoch(99),
                ),
        )
        .unwrap();
    assert!(
        read_fork_catalog(&reader, parent, SessionForkLimits::default())
            .unwrap_err()
            .to_string()
            .contains("discovery")
    );
}

// 최신 parent가 accepted suffix를 남기면 정상인 과거 initial seed도 선택 가능한 목록이 되지 않는다.
#[test]
fn historical_catalog_rejects_a_latest_uncertain_parent() {
    use crate::session_repository::{SessionForkLimits, read_fork_catalog};
    let parent = fixture_session(82);
    let directory = ForkRepositoryDirectory(env::temp_dir().join(format!(
        "yo-historical-uncertain-{}",
        crate::SessionId::new().unwrap()
    )));
    let (initial, _) = fork_two_group_bootstrap();
    let (request, _, _) = fork_private_turn(5, 1, 1);
    let mut journal = JournalRepository::new(
        LocalSessionRepository::open(&directory.0, 8 * 1024 * 1024).unwrap(),
    );
    journal.append(parent, &initial).unwrap();
    journal.append(parent, &request).unwrap();
    let reader = LocalSessionReader::open(&directory.0).unwrap();
    assert!(read_fork_catalog(&reader, parent, SessionForkLimits::default()).is_err());
}

// command가 진행 중이거나 request가 수락된 뒤 완료 증거가 없는 parent는 오래된 seed를
// 사용해 실행 가능한 child를 캡처하지 못합니다.
#[test]
fn capture_fork_rejects_active_and_uncertain_suffix() {
    let (initial, _) = fork_two_group_bootstrap();
    let (_, request) = fork_child_request();
    let active = fork_child_commit(5, vec![request.records()[0].record().clone()]);
    for suffix in [active, request] {
        let recovered = recover(&[initial.clone(), suffix]).unwrap();
        assert!(
            recovered
                .capture_fork(crate::SessionId::new().unwrap())
                .is_err()
        );
    }
}

// 상속 history와 현재 parent의 SessionCreated를 합쳐 4096개까지 캡처하고 첫 초과 항목을
// 조용히 자르지 않고 거부합니다.
#[test]
fn capture_fork_history_accepts_limit_and_rejects_first_inherited_excess() {
    for inherited_count in [4095_usize, 4096] {
        let mut records = fork_records(true);
        let JournalRecord::InitialForkSeed(seed) = &records[2] else {
            panic!("seed")
        };
        let parent = fixture_session(81);
        let history = (1..=inherited_count)
            .map(|index| {
                let sequence = JournalSequence::new(u64::try_from(index).unwrap());
                ForkHistoryEntry::new(
                    parent,
                    ForkHistoryCoordinate::Journal { sequence },
                    SequencedJournalRecord::with_journal_sequence(
                        ReplaySequence::new(sequence.get()),
                        sequence,
                        JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                            turn: TurnRef::new(
                                parent,
                                TurnId::new(sequence.get().try_into().unwrap()),
                            ),
                            outcome: TurnOutcome::Completed,
                        }),
                    ),
                )
                .unwrap()
            })
            .collect();
        let source = ForkSource::Anchor(
            ForkSourcePoint::new(
                3,
                2,
                JournalSequence::new(5001),
                JournalSequence::new(5000),
                seed.source().point().unwrap().binding().clone(),
            )
            .unwrap(),
        );
        records[2] = JournalRecord::InitialForkSeed(Box::new(
            InitialForkSeed::new(
                fixture_session(82),
                parent,
                source,
                seed.seed().clone(),
                history,
                1024 * 1024,
            )
            .unwrap(),
        ));
        let initial = fork_commit(records);
        let recovered = recover(&[decode(&encode(&initial).unwrap()).unwrap()]).unwrap();
        let captured = recovered.capture_fork(crate::SessionId::new().unwrap());
        if inherited_count == 4095 {
            let captured = captured.unwrap();
            assert_eq!(captured.history().len(), 4096);
            assert!(
                captured.history()[..inherited_count]
                    .iter()
                    .all(|entry| entry.source_session_id() == parent)
            );
            let appended = captured.history().last().unwrap();
            assert_eq!(appended.source_session_id(), fixture_session(82));
            assert!(
                matches!(appended.record().record(), JournalRecord::EventCommitted(AgentEvent::SessionCreated { session_id }) if *session_id == fixture_session(82))
            );
        } else {
            assert!(
                captured
                    .unwrap_err()
                    .to_string()
                    .contains("fork history item limit exceeded")
            );
        }
    }
}
