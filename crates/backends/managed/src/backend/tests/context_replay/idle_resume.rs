use std::{
    env, process,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use yo_core::{
    AgentCommand, AgentEvent, AgentIntent, AgentSession, ModelConnectorEvent,
    ToolApprovalRequirement, TranscriptRecord, TurnOutcome,
};

use super::{
    super::support::backend,
    fixtures::{completed_text_round, portable_summary, queue_intent, wait_for_turn_finish},
};

// idle 압축의 비축소·형식 거절 직후 종료해도 CompactContext만으로 기존 Anchor를
// 무효화하지 않는다. 실제 disk를 다시 열어 같은 replay로 다음 Turn을 완료한다.
#[test]
fn rejected_idle_compaction_survives_immediate_disk_shutdown_and_resume() {
    use std::{fs, path::PathBuf};

    use yo_core::{
        AgentControlOutcome, AgentSessionPoll, HostWorkspacePath, SessionDescriptor,
        WorkspaceHostId,
        session_repository::{
            LocalSessionReader, LocalSessionRepository, SessionWriterRepository,
            StoredSessionReader, read_stored_session, read_stored_session_continuation,
            recover_stored_session_continuation,
        },
    };
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    for case in ["nonreduction", "wrong-heading", "missing-section"] {
        let root = env::temp_dir().join(format!(
            "yo-idle-rejection-resume-{}-{}",
            process::id(),
            WorkspaceHostId::new().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Fixture(root);
        let storage = fixture.0.join("repository");
        let descriptor = SessionDescriptor::new(
            WorkspaceHostId::new().unwrap(),
            HostWorkspacePath::normalize_local(&fixture.0).unwrap(),
        )
        .unwrap();
        let session_id = descriptor.session_id();
        let body = match case {
            "nonreduction" => portable_summary(),
            "wrong-heading" => {
                portable_summary().replacen("# Context Checkpoint", "# secret-malformed-canary", 1)
            },
            "missing-section" => portable_summary()
                .replace("\n## Critical References\nNone.", "")
                .replace("Continue the current task.", "secret-malformed-canary"),
            _ => unreachable!(),
        };
        let mut summary_round = completed_text_round("idle-summary", &body);
        let Some(ModelConnectorEvent::Terminal { usage, .. }) = summary_round.last_mut() else {
            unreachable!()
        };
        *usage = yo_core::ResponsesUsage {
            input_tokens: Some(1),
            output_tokens: Some(1),
            total_tokens: Some(2),
            reasoning_tokens: None,
            cache_read_input_tokens: yo_core::CacheReadInputTokens::Unsupported,
        };
        // 고정된 1-token counter로 valid-summary case는 nonreducing이 되며, 나머지
        // case는 successor token admission 전에 잘못된 형식을 거절합니다.
        let managed = backend(
            vec![
                completed_text_round("one", "first"),
                completed_text_round("two", "second"),
                summary_round,
            ],
            ToolApprovalRequirement::Automatic,
            Arc::new(Mutex::new(0)),
        );
        let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
        repository.acquire_session_writer(session_id).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut session = AgentSession::start_cancellable_with_repository(
            managed,
            descriptor.clone(),
            repository,
            || Instant::now() >= deadline,
        )
        .unwrap()
        .unwrap();
        let transcript = session.transcript_reader();
        let mut cursor = None;
        for number in [1, 2] {
            queue_intent(
                &mut session,
                AgentIntent::submit(format!("input-{number}")).unwrap(),
            );
            wait_for_turn_finish(&mut session, &transcript, &mut cursor, number);
        }
        let reader = LocalSessionReader::open(&storage).unwrap();
        let before = read_stored_session_continuation(&reader, session_id).unwrap();
        let original_target = before.target().clone();
        let prefix = reader.read_after(session_id, None, 4096).unwrap();
        assert!(!prefix.is_empty() && prefix.len() < 4096);
        queue_intent(&mut session, AgentIntent::CompactContext { guidance: None });
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            assert_ne!(session.poll().unwrap(), AgentSessionPoll::Closed);
            if let Some(AgentControlOutcome::ContextCompactionRejected { detail }) =
                session.take_control_outcome()
            {
                assert!(detail.contains(if case == "nonreduction" {
                    "did not reduce"
                } else {
                    "required summary format"
                }));
                assert!(!detail.contains("secret-malformed-canary"));
                break;
            }
            assert!(
                Instant::now() < deadline,
                "idle rejection was not delivered"
            );
            thread::sleep(Duration::from_millis(1));
        }
        // 이 종료 전에 중간 ordinary Turn이나 새 Anchor가 continuation을 복구해서는 안 됩니다.
        session.shutdown().unwrap();
        drop(session);
        drop(transcript);
        let history = read_stored_session(&reader, session_id).unwrap();
        assert_eq!(
            history
                .records()
                .iter()
                .filter(|record| matches!(
                    record,
                    TranscriptRecord::CommandCommitted(AgentCommand::CompactContext { .. })
                ))
                .count(),
            1
        );
        assert!(
            !history
                .records()
                .iter()
                .any(|record| matches!(record, TranscriptRecord::ContextCheckpointCommitted(_)))
        );
        let after = reader.read_after(session_id, None, 8192).unwrap();
        assert!(after.len() > prefix.len() && after.len() < 8192);
        assert!(after.starts_with(&prefix));
        let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
        let continuation =
            recover_stored_session_continuation(&mut repository, session_id).unwrap();
        assert_eq!(continuation.descriptor(), &descriptor);
        assert_eq!(
            continuation.target(),
            &original_target,
            "rejected idle compaction must preserve the newest Anchor, binding, epoch and exact replay"
        );
        let managed = backend(
            vec![completed_text_round("three", "third")],
            ToolApprovalRequirement::Automatic,
            Arc::new(Mutex::new(0)),
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut session = AgentSession::start_cancellable_with_continuation(
            managed,
            continuation,
            repository,
            || Instant::now() >= deadline,
        )
        .unwrap()
        .unwrap();
        let transcript = session.transcript_reader();
        let mut cursor = transcript.read_after(None).head();
        queue_intent(&mut session, AgentIntent::submit("input-3").unwrap());
        wait_for_turn_finish(&mut session, &transcript, &mut cursor, 3);
        session.shutdown().unwrap();
        drop(session);
        drop(transcript);
        let history = read_stored_session(&reader, session_id).unwrap();
        let completed = history
            .records()
            .iter()
            .filter_map(|record| match record {
                TranscriptRecord::EventCommitted(AgentEvent::TurnFinished { turn, outcome }) => {
                    Some((*turn, outcome.clone()))
                },
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(completed.len(), 3);
        for (index, (turn, outcome)) in completed.iter().enumerate() {
            assert_eq!(turn.session_id(), session_id);
            assert_eq!(turn.turn_id().get().get(), index as u64 + 1);
            assert_eq!(*outcome, TurnOutcome::Completed);
        }
        assert!(
            !history
                .records()
                .iter()
                .any(|record| matches!(record, TranscriptRecord::ContextCheckpointCommitted(_)))
        );
    }
}
