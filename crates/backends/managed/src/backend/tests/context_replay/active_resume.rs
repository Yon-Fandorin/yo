use std::{
    env, process,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use yo_backend::BackendAdapter as AgentBackend;
use yo_core::{
    AgentEvent, AgentIntent, AgentSession, ModelConnectorEvent, ToolApprovalRequirement,
    TranscriptRecord, TurnOutcome,
};

use super::{
    super::support::{ExactAdmission, MockConnector, MockHost, binding, event_rounds, registry},
    fixtures::{
        SequenceTokenCounter, completed_summary_round, completed_text_round, portable_summary,
        private_summary_event, queue_intent, wait_for_turn_finish,
    },
};
use crate::backend::{NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices};

// 실제 worker·disk Journal·managed connector 경계를 이어 자동 압축 뒤 재개한 요청이
// checkpoint의 정확한 보존 문맥만 사용하고 요약 원본이나 private summary를 재생하지 않는지
// 검증한다.
#[test]
fn automatic_compaction_survives_disk_resume_with_exact_retained_connector_input() {
    for output_index in [0, 1] {
        check_automatic_compaction_survives_disk_resume_with_exact_retained_connector_input(
            output_index,
        );
    }
}

fn check_automatic_compaction_survives_disk_resume_with_exact_retained_connector_input(
    output_index: usize,
) {
    use std::{fs, path::PathBuf};

    use yo_core::{
        ConnectorError, HostWorkspacePath, ModelConnector, ModelConnectorCancellation,
        ModelConnectorInputItem, ModelConnectorInputRole, ModelConnectorRequest,
        ModelConnectorStreamPort, SessionDescriptor, SessionId, WorkspaceHostId,
        session_repository::{
            LocalSessionReader, LocalSessionRepository, SessionRepository, SessionWriterRepository,
            read_stored_session, recover_stored_session_continuation,
        },
    };

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    struct DiskCheckpointConnector {
        inner: MockConnector,
        storage: PathBuf,
        session_id: SessionId,
    }
    impl ModelConnector for DiskCheckpointConnector {
        fn request_url(&self) -> &str {
            self.inner.request_url()
        }
        fn tokenization_payload(
            &self,
            request: &ModelConnectorRequest,
        ) -> Result<serde_json::Value, ConnectorError> {
            self.inner.tokenization_payload(request)
        }
        fn start(
            &self,
            request: ModelConnectorRequest,
            cancellation: ModelConnectorCancellation,
        ) -> Result<Box<dyn ModelConnectorStreamPort>, ConnectorError> {
            if self.inner.requests.lock().unwrap().len() == 3 {
                // 실제 on-disk checkpoint가 존재하기 전에는 successor가 connector 경계를 통과할 수
                // 없습니다. 이 reader는 Session writer를 획득하지 않습니다.
                let reader = LocalSessionReader::open(&self.storage).unwrap();
                let history = read_stored_session(&reader, self.session_id).unwrap();
                assert_eq!(
                    history
                        .records()
                        .iter()
                        .filter(|record| matches!(
                            record,
                            TranscriptRecord::ContextCheckpointCommitted(_)
                        ))
                        .count(),
                    1,
                    "successor dispatch preceded durable checkpoint publication"
                );
            }
            self.inner.start(request, cancellation)
        }
    }
    let root = env::temp_dir().join(format!(
        "yo-managed-compaction-resume-{}-{}",
        process::id(),
        WorkspaceHostId::new().unwrap(),
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
    let config = NativeModelBackendConfig::default();
    let system_prompt = config.system_prompt.clone();
    let build_backend = |rounds, requests, counts: Vec<u64>| {
        NativeModelBackend::with_connector(
            Box::new(DiskCheckpointConnector {
                inner: MockConnector {
                    rounds: event_rounds(rounds),
                    requests,
                },
                storage: storage.clone(),
                session_id,
            }),
            binding(),
            registry(ToolApprovalRequirement::Automatic),
            NativeModelBackendServices::new(
                Box::new(yo_core::admit_standard_complete_binding),
                Some(Box::new(ExactAdmission)),
                Box::new(MockHost::default()),
                Box::new(SequenceTokenCounter::new(
                    counts,
                    Arc::new(Mutex::new(Vec::new())),
                )),
            ),
            yo_core::ModelContextProfile::new(100, 10, "test-tokenizer/v1").unwrap(),
            config.clone(),
        )
        .unwrap()
    };
    let summary = portable_summary();
    let mut summary_round = completed_summary_round("summary-once", &summary, output_index);
    summary_round.insert(1, private_summary_event());
    let Some(ModelConnectorEvent::Terminal { usage, .. }) = summary_round.last_mut() else {
        unreachable!()
    };
    *usage = yo_core::ResponsesUsage {
        input_tokens: Some(20),
        output_tokens: Some(10),
        total_tokens: Some(30),
        reasoning_tokens: None,
        cache_read_input_tokens: yo_core::CacheReadInputTokens::Unsupported,
    };
    let requests = Arc::new(Mutex::new(Vec::new()));
    let backend = build_backend(
        vec![
            completed_text_round("turn-1", "first"),
            completed_text_round("turn-2", "second"),
            summary_round,
            completed_text_round("turn-3", "third"),
        ],
        Arc::clone(&requests),
        vec![10, 10, 90, 20, 30, 30],
    );
    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
    repository.acquire_session_writer(session_id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut session = AgentSession::start_cancellable_with_repository(
        backend,
        descriptor.clone(),
        repository,
        || Instant::now() >= deadline,
    )
    .unwrap()
    .expect("initial startup is bounded");
    let transcript = session.transcript_reader();
    let mut cursor = None;
    for number in [1, 2, 3] {
        queue_intent(
            &mut session,
            AgentIntent::submit(format!("input-{number}")).unwrap(),
        );
        wait_for_turn_finish(&mut session, &transcript, &mut cursor, number);
    }
    session.shutdown().unwrap();
    drop(session);
    drop(transcript);
    assert_eq!(
        requests.lock().unwrap().len(),
        4,
        "one summary request, three ordinary requests"
    );

    let reader = LocalSessionReader::open(&storage).unwrap();
    let history = read_stored_session(&reader, session_id).unwrap();
    let checkpoints = history
        .records()
        .iter()
        .filter(|record| matches!(record, TranscriptRecord::ContextCheckpointCommitted(_)))
        .count();
    assert_eq!(checkpoints, 1, "the automatic checkpoint must be durable");
    let completed_turns = history
        .records()
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::EventCommitted(AgentEvent::TurnFinished { turn, outcome }) => {
                Some((*turn, outcome.clone()))
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(completed_turns.len(), 3);
    for (index, (turn, outcome)) in completed_turns.iter().enumerate() {
        assert_eq!(turn.session_id(), session_id);
        assert_eq!(turn.turn_id().get().get(), index as u64 + 1);
        assert_eq!(*outcome, TurnOutcome::Completed);
    }
    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
    let prefix = repository.read_after(session_id, None, 4096).unwrap();
    assert!(!prefix.is_empty() && prefix.len() < 4096);
    let continuation = recover_stored_session_continuation(&mut repository, session_id).unwrap();
    assert_eq!(continuation.descriptor(), &descriptor);
    assert_eq!(continuation.target().epoch(), 1);
    assert_eq!(continuation.target().context_epoch(), Some(2));
    let original_binding = continuation.target().binding().clone();

    let resumed_requests = Arc::new(Mutex::new(Vec::new()));
    let backend = build_backend(
        vec![completed_text_round("turn-4", "fourth")],
        Arc::clone(&resumed_requests),
        vec![40],
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut session = AgentSession::start_cancellable_with_continuation(
        backend,
        continuation,
        repository,
        || Instant::now() >= deadline,
    )
    .unwrap()
    .expect("resume startup is bounded");
    assert!(
        resumed_requests.lock().unwrap().is_empty(),
        "recovery must not dispatch model work"
    );
    let transcript = session.transcript_reader();
    let mut cursor = transcript.read_after(None).head();
    queue_intent(&mut session, AgentIntent::submit("input-4").unwrap());
    wait_for_turn_finish(&mut session, &transcript, &mut cursor, 4);
    session.shutdown().unwrap();
    drop(session);
    drop(transcript);

    let message = |role, content: &str| ModelConnectorInputItem::Message {
        role,
        content: content.to_owned(),
        refusal: None,
    };
    let expected = vec![
        message(ModelConnectorInputRole::System, &system_prompt),
        message(ModelConnectorInputRole::User, &summary),
        message(ModelConnectorInputRole::User, "input-2"),
        message(ModelConnectorInputRole::Assistant, "second"),
        message(ModelConnectorInputRole::User, "input-3"),
        message(ModelConnectorInputRole::Assistant, "third"),
        message(ModelConnectorInputRole::User, "input-4"),
    ];
    let resumed = resumed_requests.lock().unwrap();
    assert_eq!(
        resumed.len(),
        1,
        "resume must not repeat the summary or prior model requests"
    );
    assert_eq!(resumed[0].input(), expected);
    assert!(!resumed[0].contains_provider_private_input());
    drop(resumed);
    // 실행 중인 successor는 이미 같은 checkpoint를 사용했으므로, disk 복구에는
    // 완료된 응답과 새 입력만 추가하고 과거 prompt를 다시 구성하지 않습니다.
    assert_eq!(requests.lock().unwrap()[3].input(), &expected[..5]);

    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
    let after = repository.read_after(session_id, None, 8192).unwrap();
    assert!(after.len() > prefix.len() && after.len() < 8192);
    assert!(
        after.starts_with(&prefix),
        "prior encoded durable records must remain unchanged"
    );
    let final_continuation =
        recover_stored_session_continuation(&mut repository, session_id).unwrap();
    assert_eq!(final_continuation.descriptor(), &descriptor);
    assert_eq!(final_continuation.target().epoch(), 1);
    assert_eq!(final_continuation.target().context_epoch(), Some(2));
    assert!(original_binding.same_resume_identity(final_continuation.target().binding()));
    let history = read_stored_session(&reader, session_id).unwrap();
    assert_eq!(
        history
            .records()
            .iter()
            .filter(|record| matches!(record, TranscriptRecord::ContextCheckpointCommitted(_)))
            .count(),
        1
    );
    let final_turns = history
        .records()
        .iter()
        .filter_map(|record| match record {
            TranscriptRecord::EventCommitted(AgentEvent::TurnFinished { turn, outcome }) => {
                Some((*turn, outcome.clone()))
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(final_turns.len(), 4);
    assert_eq!(final_turns[3].0.session_id(), session_id);
    assert_eq!(final_turns[3].0.turn_id().get().get(), 4);
    assert_eq!(final_turns[3].1, TurnOutcome::Completed);
}
