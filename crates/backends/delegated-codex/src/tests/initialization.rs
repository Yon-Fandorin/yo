use std::time::Duration;

use serde_json::json;
use yo_core::{
    AgentCommand, BackendBindingEvidence, BackendCommandEvidence, BackendFailureKind,
    BackendIdentity, BackendPoll, ContinuationStrategy,
};

use super::{
    super::client::AppServerClient,
    support::{FakePeer, backend, initialize_response, session, thread_start_response},
};

fn resume_binding(thread_id: &str) -> BackendBindingEvidence {
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
        ContinuationStrategy::BackendManagedState,
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
        yo_core::ContinuationStrategy::BackendManagedState
    );

    let sent = sent.0.borrow();
    assert_eq!(sent[0]["method"], "initialize");
    assert_eq!(sent[1], json!({ "method": "initialized" }));
    assert_eq!(sent[2]["id"], 2);
    assert_eq!(sent[2]["method"], "thread/start");
    assert_eq!(sent[2]["params"]["cwd"], "/workspace");
    assert!(sent[2]["params"].get("ephemeral").is_none());
}

// 연결 대상 검증은 initialize handshake까지만 수행하고 thread/start나 thread/resume을
// 보내지 않아, 기본값을 고르는 과정이 새 backend Session을 만들지 않음을 확인합니다.
#[test]
fn verifies_the_handshake_without_creating_a_thread() {
    let (mut backend, sent) = backend([]);

    backend.verify().unwrap();

    let sent = sent.0.borrow();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0]["method"], "initialize");
    assert_eq!(sent[1], json!({ "method": "initialized" }));
}

// 로컬 최신 Codex 0.149 userAgent도 검증된 app-server line으로 수락하고 initialized
// 알림까지 완료해, 최신 설치가 호환성 allowlist에서 막히지 않음을 확인합니다.
#[test]
fn latest_codex_minor_completes_initialization() {
    let (peer, sent) = FakePeer::new([initialize_response(1, "0.149.0")]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));

    let initialize = client.initialize().unwrap();

    assert_eq!(initialize.user_agent, "codex_cli_rs/0.149.0 (test)");
    let sent = sent.0.borrow();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1], json!({ "method": "initialized" }));
}

// 저장된 locator로 재개할 때 새 thread를 만들지 않고 `thread/resume` 한 번만 보내며,
// app-server 버전이 달라도 thread·Session·model·provider 신원이 같으면 같은 binding이다.
#[test]
fn resumes_one_verified_thread_without_starting_another() {
    let (mut backend, sent) = backend([thread_start_response(2, "thread-a")]);
    let evidence = backend
        .resume_binding(session(1), &resume_binding("thread-a"))
        .unwrap();

    assert_eq!(evidence.backend_version(), "codex_cli_rs/0.146.0 (test)");
    let sent = sent.0.borrow();
    assert_eq!(sent[2]["method"], "thread/resume");
    assert_eq!(sent[2]["params"], json!({ "threadId": "thread-a" }));
    assert!(
        sent.iter()
            .all(|message| message["method"] != "thread/start")
    );
}

// thread/resume 직후 app-server가 재생하는 과거 누적 사용량은 현재 프로세스의 Yo Turn과
// 대응시킬 근거가 없으므로 임의 귀속하거나 Session을 깨지 않고 관찰 경계 밖으로 둔다.
#[test]
fn ignores_replayed_usage_without_a_current_turn_binding() {
    let replayed_usage = json!({
        "method": "thread/tokenUsage/updated",
        "params": {
            "threadId": "thread-a",
            "turnId": "historical-turn",
            "tokenUsage": {
                "last": {
                    "inputTokens": 12,
                    "cachedInputTokens": 8,
                    "outputTokens": 3,
                    "reasoningOutputTokens": 1,
                    "totalTokens": 15
                },
                "total": {
                    "inputTokens": 120,
                    "cachedInputTokens": 80,
                    "outputTokens": 30,
                    "reasoningOutputTokens": 10,
                    "totalTokens": 150
                },
                "modelContextWindow": 200000
            }
        }
    });
    let (mut backend, _) = backend([replayed_usage, thread_start_response(2, "thread-a")]);

    backend
        .resume_binding(session(1), &resume_binding("thread-a"))
        .unwrap();

    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
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
        .resume_binding(session(1), &resume_binding("thread-a"))
        .unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Session);
    assert_eq!(sent.0.borrow()[2]["method"], "thread/resume");
}

// 검증하지 않은 Codex minor 버전이면 initialized 알림이나 Session 명령을 보내기 전에
// Initialization 실패로 연결을 중단하고, 검증된 최신 0.149는 이 경로에 들지 않게 한다.
#[test]
fn incompatible_version_fails_during_initialization() {
    let (peer, sent) = FakePeer::new([initialize_response(1, "0.150.0")]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));

    let failure = match client.initialize() {
        Ok(_) => panic!("an unverified Codex version must not initialize"),
        Err(failure) => failure,
    };

    assert_eq!(failure.kind(), BackendFailureKind::Initialization);
    assert_eq!(sent.0.borrow().len(), 1);
}
