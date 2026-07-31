use super::*;

// 백엔드 adapter가 semantic ModelWork 본문으로 공개한 text는 live 화면에서만 보였다가
// 사라지지 않고 segment와 terminal seal로 저장되어 재개 가능한 Transcript가 되어야 한다.
#[test]
fn model_work_text_is_persisted_as_a_sealed_message() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let repository = SharedRepository::default();
    let observed = Arc::clone(&repository.state);
    let mut journal = SessionJournal::with_repository(Box::new(repository));

    journal.append_events(&[AgentEvent::ActivityStarted {
        activity,
        kind: ActivityKind::ModelWork,
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity,
        update: ActivityUpdate::TextDelta("inspect the journal".to_owned()),
    }]);
    journal.append_events(&[AgentEvent::ActivityFinished {
        activity,
        outcome: ActivityOutcome::Completed,
    }]);

    let records = observed
        .lock()
        .unwrap()
        .entries
        .iter()
        .flat_map(|entry| decode(entry.record().payload()).unwrap().records().to_vec())
        .collect::<Vec<_>>();
    let terminal = records
        .iter()
        .find_map(|entry| match entry.record() {
            super::super::codec::JournalRecord::MessageEnded(terminal) => Some(terminal),
            _ => None,
        })
        .expect("the observable ModelWork text is durably sealed");
    assert_eq!(
        terminal.final_segment().unwrap().text(),
        "inspect the journal"
    );
    assert_eq!(terminal.ended().segment_count(), 1);
    assert_eq!(terminal.ended().utf8_bytes(), 19);
}

// live TextDelta와 권위 있는 TextSnapshot은 frontend Journal에는 즉시 남지만 저장소에는
// raw update로 기록하지 않고, Activity 종료 시 최종 revision과 seal을 한 envelope로 남긴다.
#[test]
fn persistent_journal_stores_the_final_message_revision_instead_of_raw_updates() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let repository = SharedRepository::default();
    let observed = Arc::clone(&repository.state);
    let mut journal = SessionJournal::with_repository(Box::new(repository));

    journal.append_committed_command(
        AgentCommand::CreateSession { session_id },
        &[AgentEvent::SessionCreated { session_id }],
    );
    journal.append_events(&[AgentEvent::ActivityStarted {
        activity,
        kind: ActivityKind::AgentMessage,
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity,
        update: ActivityUpdate::TextDelta("draft".to_owned()),
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity,
        update: ActivityUpdate::TextSnapshot("final answer".to_owned()),
    }]);
    journal.append_events(&[AgentEvent::ActivityFinished {
        activity,
        outcome: ActivityOutcome::Completed,
    }]);

    let state = observed.lock().unwrap();
    let records = state
        .entries
        .iter()
        .flat_map(|entry| decode(entry.record().payload()).unwrap().records().to_vec())
        .collect::<Vec<_>>();
    assert!(records.iter().all(|entry| {
        !matches!(
            entry.record(),
            super::super::codec::JournalRecord::EventCommitted(AgentEvent::ActivityUpdated { .. })
        )
    }));
    let terminal = records
        .iter()
        .find_map(|entry| match entry.record() {
            super::super::codec::JournalRecord::MessageEnded(terminal) => Some(terminal),
            _ => None,
        })
        .expect("the final message revision is sealed");
    assert_eq!(terminal.ended().revision(), 2);
    assert_eq!(terminal.final_segment().unwrap().text(), "final answer");
}

// 짧은 text가 아직 메모리 buffer에 있을 때 다른 Activity가 시작되면 text segment를 먼저
// 강제 저장하고 그 다음 사건을 기록해야 한다. 이때 durable cutoff는 segment 개수가 아니라
// frontend가 보는 마지막 semantic JournalSequence와 정확히 같아야 한다.
#[test]
fn forces_buffered_text_before_a_non_text_boundary_with_one_semantic_cutoff() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let message = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let tool = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(2).unwrap()));
    let repository = SharedRepository::default();
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
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity: message,
        update: ActivityUpdate::TextDelta("before tool".to_owned()),
    }]);
    journal.append_events(&[AgentEvent::ActivityStarted {
        activity: tool,
        kind: ActivityKind::ToolCall,
    }]);

    let state = observed.lock().unwrap();
    let last = state
        .entries
        .last()
        .expect("the boundary commit is durable");
    let commit = decode(last.record().payload()).unwrap();
    assert!(matches!(
        commit.records()[0].record(),
        super::super::codec::JournalRecord::MessageSegment(segment)
            if segment.text() == "before tool"
    ));
    assert!(matches!(
        commit.records()[1].record(),
        super::super::codec::JournalRecord::EventCommitted(
            AgentEvent::ActivityStarted { activity, .. }
        ) if *activity == tool
    ));
    assert_eq!(
        last.record().journal_cutoff(),
        journal.transcript_reader().head_sequence()
    );
}

// size bound에서 이미 segment가 durable해진 message 뒤에 다른 사건이 와도 전체 commit을
// 다시 재생할 수 있어야 한다. 이는 동시 Activity가 있는 실제 provider 흐름이 영구적인
// integrity gap으로 굳지 않는지를 검증한다.
#[test]
fn recovers_after_a_size_forced_segment_and_an_unrelated_event() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let message = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let tool = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(2).unwrap()));
    let repository = SharedRepository::default();
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
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity: message,
        update: ActivityUpdate::TextDelta("x".repeat(16 * 1024)),
    }]);
    journal.append_events(&[AgentEvent::ActivityStarted {
        activity: tool,
        kind: ActivityKind::ToolCall,
    }]);

    let commits = observed
        .lock()
        .unwrap()
        .entries
        .iter()
        .map(|entry| decode(entry.record().payload()).unwrap())
        .collect::<Vec<_>>();
    let recovered = recover(&commits).expect("the interleaved durable history recovers");
    assert!(recovered.recovery_commit().is_some());
}

// text update가 전혀 없는 message도 ActivityFinished가 관찰되면 zero-byte MessageEnded를
// 남겨야 완료와 crash 중단을 구분할 수 있다. 다시 읽었을 때 recovery seal이 필요 없어야
// 정상 종료가 durable authority로 확정되었음을 증명한다.
#[test]
fn empty_message_is_sealed_with_a_zero_byte_terminal() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let message = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let repository = SharedRepository::default();
    let observed = Arc::clone(&repository.state);
    let mut journal = SessionJournal::with_repository(Box::new(repository));

    journal.append_events(&[AgentEvent::ActivityStarted {
        activity: message,
        kind: ActivityKind::AgentMessage,
    }]);
    journal.append_events(&[AgentEvent::ActivityFinished {
        activity: message,
        outcome: ActivityOutcome::Completed,
    }]);

    let commits = observed
        .lock()
        .unwrap()
        .entries
        .iter()
        .map(|entry| decode(entry.record().payload()).unwrap())
        .collect::<Vec<_>>();
    let recovered = recover(&commits).unwrap();
    assert!(recovered.recovery_commit().is_none());
    let terminal = recovered
        .records()
        .iter()
        .find_map(|entry| match entry.record() {
            super::super::codec::JournalRecord::MessageEnded(terminal) => Some(terminal.ended()),
            _ => None,
        })
        .unwrap();
    assert_eq!(terminal.segment_count(), 0);
    assert_eq!(terminal.utf8_bytes(), 0);
}

// revision 1 text가 이미 durable한 뒤 권위 있는 empty snapshot이 오면 그 옛 본문을
// 되살리지 않는다. revision 2의 zero-byte seal을 저장하고 integrity gap 없이 복구한다.
#[test]
fn durable_message_can_be_authoritatively_replaced_with_empty_text() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let message = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let repository = SharedRepository::default();
    let observed = Arc::clone(&repository.state);
    let mut journal = SessionJournal::with_repository(Box::new(repository));

    journal.append_events(&[AgentEvent::ActivityStarted {
        activity: message,
        kind: ActivityKind::AgentMessage,
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity: message,
        update: ActivityUpdate::TextDelta("x".repeat(16 * 1024)),
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity: message,
        update: ActivityUpdate::TextSnapshot(String::new()),
    }]);
    journal.append_events(&[AgentEvent::ActivityFinished {
        activity: message,
        outcome: ActivityOutcome::Completed,
    }]);

    assert!(matches!(
        journal.transcript_reader().durability(),
        JournalDurability::Durable { .. }
    ));
    let commits = observed
        .lock()
        .unwrap()
        .entries
        .iter()
        .map(|entry| decode(entry.record().payload()).unwrap())
        .collect::<Vec<_>>();
    let recovered = recover(&commits).unwrap();
    let terminal = recovered
        .records()
        .iter()
        .find_map(|entry| match entry.record() {
            super::super::codec::JournalRecord::MessageEnded(terminal) => Some(terminal.ended()),
            _ => None,
        })
        .unwrap();

    assert_eq!(terminal.revision(), 2);
    assert_eq!(terminal.segment_count(), 0);
    assert_eq!(terminal.utf8_bytes(), 0);
    assert!(recovered.recovery_commit().is_none());
}

// 이미 저장된 본문을 empty snapshot으로 교체한 뒤 unrelated event가 경계를 만들면,
// 종료 전 crash가 나도 옛 본문이 되살아나지 않도록 empty revision을 먼저 저장해야 한다.
#[test]
fn empty_replacement_is_durable_before_a_later_non_text_event() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let message = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    let tool = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(2).unwrap()));
    let repository = SharedRepository::default();
    let observed = Arc::clone(&repository.state);
    let mut journal = SessionJournal::with_repository(Box::new(repository));

    journal.append_events(&[AgentEvent::ActivityStarted {
        activity: message,
        kind: ActivityKind::AgentMessage,
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity: message,
        update: ActivityUpdate::TextDelta("x".repeat(16 * 1024)),
    }]);
    journal.append_events(&[AgentEvent::ActivityUpdated {
        activity: message,
        update: ActivityUpdate::TextSnapshot(String::new()),
    }]);
    journal.append_events(&[AgentEvent::ActivityStarted {
        activity: tool,
        kind: ActivityKind::ToolCall,
    }]);

    let commits = observed
        .lock()
        .unwrap()
        .entries
        .iter()
        .map(|entry| decode(entry.record().payload()).unwrap())
        .collect::<Vec<_>>();
    let recovered = recover(&commits).unwrap();
    assert!(recovered.records().iter().any(|entry| {
        matches!(
            entry.record(),
            super::super::codec::JournalRecord::MessageReset(reset)
                if reset.activity() == message && reset.revision() == 2
        )
    }));
    let seal = recovered
        .recovery_commit()
        .expect("the still-open activities receive recovery seals");
    assert!(seal.records().iter().any(|entry| {
        matches!(
            entry.record(),
            super::super::codec::JournalRecord::MessageEnded(terminal)
                if terminal.ended().activity() == message
                    && terminal.ended().revision() == 2
                    && terminal.ended().utf8_bytes() == 0
        )
    }));
}
