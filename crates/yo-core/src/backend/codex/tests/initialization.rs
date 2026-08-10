use std::time::Duration;

use serde_json::json;

use super::{
    super::client::AppServerClient,
    support::{FakePeer, backend, initialize_response, session, thread_start_response},
};
use crate::{
    AgentCommand, BackendBindingEvidence, BackendCommandEvidence, BackendFailureKind,
    BackendIdentity, BackendResumeTarget, SessionId,
};

fn resume_target(session_id: SessionId, thread_id: &str) -> BackendResumeTarget {
    BackendResumeTarget::new(
        session_id,
        1,
        BackendBindingEvidence::new(
            "codex-app-server",
            "codex_cli_rs/0.145.0 (recorded)",
            BackendIdentity::new(
                "codex.app-server/thread-binding/v1",
                json!({ "sessionId": thread_id, "threadId": thread_id }).to_string(),
            ),
            BackendIdentity::new(
                "codex.app-server/model-and-provider/v1",
                json!({ "model": "gpt-test", "provider": "openai" }).to_string(),
            ),
            BackendIdentity::new("codex.app-server/thread-locator/v1", thread_id),
            crate::ContinuationStrategy::BackendManagedState,
        ),
        crate::JournalSequence::new(1),
    )
}

// 초기화가 성공하면 initialize 요청 다음에 initialized 알림을 정확한 순서로 보내고,
// 이후 첫 Session RPC가 다음 request id와 설정된 작업 디렉토리를 사용하는지 확인한다.
#[test]
fn initializes_before_starting_a_thread() {
    let (mut backend, sent) = backend([thread_start_response(2, "thread-a")]);

    let evidence = backend
        .execute_command(AgentCommand::CreateSession {
            session_id: session(1),
        })
        .unwrap();

    let BackendCommandEvidence::BindingOpened(evidence) = evidence else {
        panic!("thread/start acceptance must expose binding evidence");
    };
    assert_eq!(evidence.backend_version(), "codex_cli_rs/0.146.0 (test)");
    assert!(evidence.model_identity().value().contains("gpt-test"));
    assert!(evidence.model_identity().value().contains("openai"));
    assert_eq!(
        evidence.continuation_strategy(),
        crate::ContinuationStrategy::BackendManagedState
    );

    let sent = sent.0.borrow();
    assert_eq!(sent[0]["method"], "initialize");
    assert_eq!(sent[1], json!({ "method": "initialized" }));
    assert_eq!(sent[2]["id"], 2);
    assert_eq!(sent[2]["method"], "thread/start");
    assert_eq!(sent[2]["params"]["cwd"], "/workspace");
    assert!(sent[2]["params"].get("ephemeral").is_none());
}

// 저장된 locator로 재개할 때 새 thread를 만들지 않고 `thread/resume` 한 번만 보내며,
// app-server 버전이 달라도 thread·Session·model·provider 신원이 같으면 같은 binding이다.
#[test]
fn resumes_one_verified_thread_without_starting_another() {
    let (mut backend, sent) = backend([thread_start_response(2, "thread-a")]);
    let target = resume_target(session(1), "thread-a");

    let evidence = backend.resume_session(&target).unwrap();

    assert_eq!(evidence.backend_version(), "codex_cli_rs/0.146.0 (test)");
    let sent = sent.0.borrow();
    assert_eq!(sent[2]["method"], "thread/resume");
    assert_eq!(sent[2]["params"], json!({ "threadId": "thread-a" }));
    assert!(
        sent.iter()
            .all(|message| message["method"] != "thread/start")
    );
}

// locator가 가리킨 thread라도 반환된 model 신원이 저장된 binding과 다르면 같은 epoch로
// 가장하지 않고 입력이 허용되기 전 Session 재개 실패로 닫는다.
#[test]
fn rejects_a_resumed_thread_with_different_model_identity() {
    let response = json!({
        "id": 2,
        "result": {
            "thread": { "id": "thread-a", "sessionId": "thread-a" },
            "model": "different-model",
            "modelProvider": "openai"
        }
    });
    let (mut backend, sent) = backend([response]);

    let failure = backend
        .resume_session(&resume_target(session(1), "thread-a"))
        .unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Session);
    assert_eq!(sent.0.borrow()[2]["method"], "thread/resume");
}

// 검증하지 않은 Codex minor 버전이면 initialized 알림이나 Session 명령을 보내기 전에
// Initialization 실패로 연결을 중단하고, 새로 검증한 0.146은 이 경로에 들지 않게 한다.
#[test]
fn incompatible_version_fails_during_initialization() {
    let (peer, sent) = FakePeer::new([initialize_response(1, "0.147.0")]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));

    let failure = match client.initialize() {
        Ok(_) => panic!("an unverified Codex version must not initialize"),
        Err(failure) => failure,
    };

    assert_eq!(failure.kind(), BackendFailureKind::Initialization);
    assert_eq!(sent.0.borrow().len(), 1);
}
