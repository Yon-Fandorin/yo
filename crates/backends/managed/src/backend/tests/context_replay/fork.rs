use std::{
    env, process,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use yo_backend::BackendAdapter as AgentBackend;
use yo_core::{
    AgentIntent, AgentSession, ToolApprovalRequirement, ToolExecution, ToolExecutionError,
    ToolExecutionHost, ToolExecutionRequest, ToolId,
};

use super::{
    super::support::{
        ExactAdmission, FixedTokenCounter, MockConnector, binding, event_rounds, registry,
    },
    fixtures::{completed_text_round, queue_intent, wait_for_turn_finish},
};
use crate::backend::{NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices};

// 실제 부모 Journal에서 준비한 child를 새 managed backend로 시작해 inherited input을 한 번만
// 전송한다. 준비는 connector/tool을 호출하지 않고 부모 저장 bytes를 바꾸지 않는다.
#[test]
fn prepared_managed_fork_resumes_exact_context_once_without_touching_parent() {
    use std::{fs, path::PathBuf};

    use yo_core::{
        HostWorkspacePath, ModelConnectorInputItem, ModelConnectorInputRole, ModelContextProfile,
        SessionDescriptor, WorkspaceHostId, admit_standard_complete_binding,
        session_repository::{
            LocalSessionReader, LocalSessionRepository, SessionWriterRepository,
            StoredSessionReader, read_stored_session_continuation,
        },
    };
    struct ForkTestHost(Arc<Mutex<usize>>);
    impl ToolExecutionHost for ForkTestHost {
        fn identity(&self) -> &str {
            "test-host-v1"
        }
        fn is_available(&self, _tool: &ToolId) -> bool {
            true
        }
        fn start(
            &mut self,
            _request: ToolExecutionRequest,
        ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
            *self.0.lock().unwrap() += 1;
            Err(ToolExecutionError::new(
                "unexpected tool execution in text-only fork fixture",
            ))
        }
        fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
            Ok(())
        }
    }
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let root = env::temp_dir().join(format!(
        "yo-managed-fork-{}",
        WorkspaceHostId::new().unwrap()
    ));
    fs::create_dir(&root).unwrap();
    let fixture = Fixture(root);
    let storage = fixture.0.join("repository");
    let host = WorkspaceHostId::new().unwrap();
    let workspace = HostWorkspacePath::normalize_local(&fixture.0).unwrap();
    let parent_descriptor = SessionDescriptor::new(host, workspace.clone()).unwrap();
    let parent_id = parent_descriptor.session_id();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let tool_starts = Arc::new(Mutex::new(0));
    let make_backend = |rounds| {
        NativeModelBackend::with_connector(
            Box::new(MockConnector {
                rounds: event_rounds(rounds),
                requests: Arc::clone(&requests),
            }),
            binding(),
            registry(ToolApprovalRequirement::Automatic),
            NativeModelBackendServices::new(
                Box::new(admit_standard_complete_binding),
                Some(Box::new(ExactAdmission)),
                Box::new(ForkTestHost(Arc::clone(&tool_starts))),
                Box::new(FixedTokenCounter(1)),
            ),
            ModelContextProfile::new(1000, 10, "test-tokenizer/v1").unwrap(),
            NativeModelBackendConfig::default(),
        )
        .unwrap()
    };
    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
    repository.acquire_session_writer(parent_id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut parent = AgentSession::start_cancellable_with_repository(
        make_backend(vec![completed_text_round("parent", "remembered")]),
        parent_descriptor,
        repository,
        || Instant::now() >= deadline,
    )
    .unwrap()
    .unwrap();
    let transcript = parent.transcript_reader();
    queue_intent(&mut parent, AgentIntent::submit("parent input").unwrap());
    wait_for_turn_finish(&mut parent, &transcript, &mut None, 1);
    parent.shutdown().unwrap();
    drop(parent);
    drop(transcript);
    let reader = LocalSessionReader::open(&storage).unwrap();
    let before = reader.read_session(parent_id).unwrap();
    let continuation = read_stored_session_continuation(&reader, parent_id).unwrap();
    let candidate = make_backend(vec![completed_text_round("child", "child answer")]);
    let child = SessionDescriptor::new(host, workspace).unwrap();
    let child_id = child.session_id();
    let prepared = candidate.prepare_exact_fork(&continuation, child).unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(*tool_starts.lock().unwrap(), 0);
    assert!(candidate.session.is_none());
    assert_eq!(
        prepared.target().model_replay(),
        continuation.target().model_replay()
    );
    assert_ne!(
        prepared.target().binding().session_locator(),
        continuation.target().binding().session_locator()
    );
    // 실제 startup에서 사용 불가능한 candidate를 재사용한 준비도 명확히 거절한다.
    let extra_child = SessionDescriptor::new(
        host,
        HostWorkspacePath::normalize_local(&fixture.0).unwrap(),
    )
    .unwrap();
    let mut repository = LocalSessionRepository::open(&storage, 16 * 1024 * 1024).unwrap();
    repository.acquire_session_writer(child_id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut child_session =
        AgentSession::start_cancellable_with_continuation(candidate, prepared, repository, || {
            Instant::now() >= deadline
        })
        .unwrap()
        .unwrap();
    assert_eq!(
        requests.lock().unwrap().len(),
        1,
        "resume must not issue a model request"
    );
    let child_transcript = child_session.transcript_reader();
    queue_intent(
        &mut child_session,
        AgentIntent::submit("child input").unwrap(),
    );
    wait_for_turn_finish(&mut child_session, &child_transcript, &mut None, 1);
    child_session.shutdown().unwrap();
    drop(child_session);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let expected = vec![
        ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::System,
            content: NativeModelBackendConfig::default().system_prompt,
            refusal: None,
        },
        ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: "parent input".into(),
            refusal: None,
        },
        ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::Assistant,
            content: "remembered".into(),
            refusal: None,
        },
        ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: "child input".into(),
            refusal: None,
        },
    ];
    assert_eq!(requests[1].input(), expected);
    assert_eq!(*tool_starts.lock().unwrap(), 0);
    assert_eq!(reader.read_session(parent_id).unwrap(), before);
    let child_recovered = read_stored_session_continuation(&reader, child_id).unwrap();
    assert_eq!(child_recovered.target().epoch(), 1);
    assert_eq!(child_recovered.target().model_replay().items().len(), 4);
    let mut closed = make_backend(Vec::new());
    closed.shutdown().unwrap();
    assert!(
        closed
            .prepare_exact_fork(&continuation, extra_child)
            .is_err()
    );
}
