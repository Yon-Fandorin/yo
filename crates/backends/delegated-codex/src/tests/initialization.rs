use std::time::Duration;

use serde_json::json;
use yo_core::{
    AccountId, AgentCommand, BackendBindingEvidence, BackendCommandEvidence, BackendFailureKind,
    BackendIdentity, BackendPoll, ContinuationStrategy, HostId, ModelId, UserInput,
    derive_host_account_id,
};

use super::{
    super::{Backend, client::AppServerClient},
    support::{
        FakePeer, backend, backend_with_profile, initialize_response, session,
        thread_start_response, turn,
    },
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

fn read_only_resume_binding(thread_id: &str) -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        "codex-app-server",
        "codex_cli_rs/0.145.0 (recorded)",
        BackendIdentity::new(
            "codex.app-server/thread-binding/v1alpha1",
            json!({
                "executionProfile": "yo.delegated-review-execution/v1alpha1",
                "sessionId": thread_id,
                "threadId": thread_id,
            })
            .to_string(),
        ),
        BackendIdentity::new(
            "codex.app-server/model-and-provider/v1",
            json!({ "model": "gpt-test", "provider": "openai" }).to_string(),
        ),
        BackendIdentity::new("codex.app-server/thread-locator/v1", thread_id),
        ContinuationStrategy::BackendManagedState,
    )
}

fn rebind_source(thread_id: &str, model: &str) -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        "codex-app-server",
        "codex_cli_rs/0.146.0 (recorded)",
        BackendIdentity::new(
            "codex.app-server/thread-binding/v2",
            json!({
                "accountId": "account-test",
                "sessionId": thread_id,
                "threadId": thread_id,
            })
            .to_string(),
        ),
        BackendIdentity::new(
            "codex.app-server/model-and-provider/v1",
            json!({ "model": model, "provider": "openai" }).to_string(),
        ),
        BackendIdentity::new("codex.app-server/thread-locator/v1", thread_id),
        ContinuationStrategy::BackendManagedState,
    )
}

// 초기화가 성공하면 initialize 요청 다음에 initialized 알림을 정확한 순서로 보내고,
// 이후 첫 Session RPC가 다음 request id와 설정된 작업 디렉토리를 사용하는지 확인한다.
#[test]
fn initializes_before_starting_a_thread() {
    let (peer, sent) = FakePeer::new([
        initialize_response(1, "0.146.0"),
        json!({
            "id": 2,
            "result": {
                "account": {
                    "type": "chatgpt",
                    "email": "person@example.test",
                    "planType": "pro"
                }
            }
        }),
        thread_start_response(3, "thread-a"),
    ]);
    let client = AppServerClient::new(peer, Duration::from_secs(1));
    let mut backend = Backend::new_uninitialized(client, "/workspace".into(), false, None);

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
        evidence.binding_identity().schema(),
        "codex.app-server/thread-binding/v2"
    );
    let account =
        derive_host_account_id(&HostId::codex(), &[("email", "person@example.test")]).unwrap();
    assert!(
        evidence
            .binding_identity()
            .value()
            .contains(account.as_str())
    );
    assert_eq!(
        evidence.continuation_strategy(),
        yo_core::ContinuationStrategy::BackendManagedState
    );

    let sent = sent.0.borrow();
    assert_eq!(sent[0]["method"], "initialize");
    assert_eq!(sent[1], json!({ "method": "initialized" }));
    assert_eq!(sent[2]["id"], 2);
    assert_eq!(sent[2]["method"], "account/read");
    assert_eq!(sent[3]["id"], 3);
    assert_eq!(sent[3]["method"], "thread/start");
    assert_eq!(sent[3]["params"]["cwd"], "/workspace");
    assert!(sent[3]["params"].get("ephemeral").is_none());
}

// API-key authentication has no stable account identifier, but ordinary Codex use remains
// available through the legacy non-rebindable binding instead of failing Session creation.
#[test]
fn api_key_account_creates_a_legacy_non_rebindable_binding() {
    let (peer, sent) = FakePeer::new([
        initialize_response(1, "0.146.0"),
        json!({"id": 2, "result": {"account": {"type": "apiKey"}}}),
        thread_start_response(3, "thread-a"),
    ]);
    let client = AppServerClient::new(peer, Duration::from_secs(1));
    let mut backend = Backend::new_uninitialized(client, "/workspace".into(), false, None);

    let evidence = backend
        .execute_command(AgentCommand::CreateSession {
            session_id: session(1),
        })
        .unwrap();

    let BackendCommandEvidence::BindingOpened(evidence) = evidence else {
        panic!("thread/start acceptance must expose binding evidence");
    };
    assert_eq!(
        evidence.binding_identity().schema(),
        "codex.app-server/thread-binding/v1"
    );
    assert!(!evidence.binding_identity().value().contains("accountId"));
    assert_eq!(sent.0.borrow()[3]["method"], "thread/start");
}

// A legacy binding remains resumable under API-key authentication because resume preserves the
// old schema and does not upgrade missing account evidence into native-rebind eligibility.
#[test]
fn api_key_account_resumes_a_legacy_binding() {
    let (peer, sent) = FakePeer::new([
        initialize_response(1, "0.146.0"),
        json!({"id": 2, "result": {"account": {"type": "apiKey"}}}),
        thread_start_response(3, "thread-a"),
    ]);
    let client = AppServerClient::new(peer, Duration::from_secs(1));
    let mut backend = Backend::new_uninitialized(client, "/workspace".into(), false, None);

    let evidence = backend
        .resume_binding(session(1), &resume_binding("thread-a"))
        .unwrap();

    assert_eq!(
        evidence.binding_identity().schema(),
        "codex.app-server/thread-binding/v1"
    );
    assert_eq!(sent.0.borrow()[3]["method"], "thread/resume");
}

// API-key authentication cannot prove same-account mutation, so an otherwise valid legacy
// Session is rejected before thread/fork while ordinary create and resume remain available.
#[test]
fn api_key_account_rejects_native_rebind_before_fork() {
    let (peer, sent) = FakePeer::new([
        initialize_response(1, "0.146.0"),
        json!({"id": 2, "result": {"account": {"type": "apiKey"}}}),
    ]);
    let client = AppServerClient::new(peer, Duration::from_secs(1));
    let mut backend = Backend::new_uninitialized(
        client,
        "/workspace".into(),
        false,
        Some((
            AccountId::new("account-test").unwrap(),
            ModelId::new("gpt-new").unwrap(),
        )),
    );

    let failure = backend
        .rebind_model(session(1), &resume_binding("thread-a"))
        .unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Unsupported);
    assert!(sent.0.borrow().iter().all(|message| {
        message.get("method").and_then(serde_json::Value::as_str) != Some("thread/fork")
    }));
}

// 같은 verified account의 exact model target은 공개 thread/fork model 파라미터 한 번으로
// 새 thread를 만들고, distinct locator와 target model을 account-bearing binding에 고정합니다.
#[test]
fn forks_provider_state_into_an_exact_model_binding() {
    let forked = json!({
        "id": 2,
        "result": {
            "thread": { "id": "thread-b", "sessionId": "session-b" },
            "model": "gpt-new",
            "modelProvider": "openai"
        }
    });
    let (mut backend, sent) = backend([forked]);
    backend.model_rebind_target = Some((
        AccountId::new("account-test").unwrap(),
        ModelId::new("gpt-new").unwrap(),
    ));

    let evidence = backend
        .rebind_model(session(1), &rebind_source("thread-a", "gpt-old"))
        .unwrap();

    assert_eq!(sent.0.borrow()[2]["method"], "thread/fork");
    assert_eq!(
        sent.0.borrow()[2]["params"],
        json!({
            "threadId": "thread-a",
            "model": "gpt-new",
            "cwd": "/workspace",
        })
    );
    assert_eq!(evidence.session_locator().value(), "thread-b");
    assert!(evidence.model_identity().value().contains("gpt-new"));
    assert!(evidence.binding_identity().value().contains("account-test"));
}

// source binding에 account가 없거나 선택 account가 다르면 private RPC를 추측하지 않고
// thread/fork 전에 닫아 기존 Session을 그대로 유지할 수 있게 합니다.
#[test]
fn rejects_legacy_or_cross_account_model_rebind_before_fork() {
    let (mut legacy, legacy_sent) = backend([]);
    legacy.model_rebind_target = Some((
        AccountId::new("account-test").unwrap(),
        ModelId::new("gpt-new").unwrap(),
    ));
    let failure = legacy
        .rebind_model(session(1), &resume_binding("thread-a"))
        .unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Unsupported);
    assert_eq!(legacy_sent.0.borrow().len(), 2);

    let (mut cross_account, cross_account_sent) = backend([]);
    cross_account.model_rebind_target = Some((
        AccountId::new("account-other").unwrap(),
        ModelId::new("gpt-new").unwrap(),
    ));
    let failure = cross_account
        .rebind_model(session(1), &rebind_source("thread-a", "gpt-old"))
        .unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Session);
    assert_eq!(cross_account_sent.0.borrow().len(), 2);
}

// planType만 같은 별도 로그인은 ordinary Session에는 쓸 수 있어도 account identity가
// 아니므로 source thread를 대상으로 thread/fork를 전송하지 않습니다.
#[test]
fn rejects_plan_only_account_before_model_rebind_fork() {
    let (peer, sent) = FakePeer::new([
        initialize_response(1, "0.146.0"),
        json!({
            "id": 2,
            "result": {"account": {"type": "chatgpt", "planType": "pro"}}
        }),
    ]);
    let client = AppServerClient::new(peer, Duration::from_secs(1));
    let mut backend = Backend::new_uninitialized(
        client,
        "/workspace".into(),
        false,
        Some((
            AccountId::new("account-test").unwrap(),
            ModelId::new("gpt-new").unwrap(),
        )),
    );

    let failure = backend
        .rebind_model(session(1), &rebind_source("thread-a", "gpt-old"))
        .unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Session);
    let sent = sent.0.borrow();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[2]["method"], "account/read");
    assert!(
        sent.iter()
            .all(|message| message["method"] != "thread/fork")
    );
}

// app-server가 요청과 다른 model이나 source와 같은 thread를 반환하면 candidate 증거를
// publish하지 않아 runtime의 원자적 old-or-new 경계를 보존합니다.
#[test]
fn rejects_inexact_thread_fork_result() {
    let response = json!({
        "id": 2,
        "result": {
            "thread": { "id": "thread-a", "sessionId": "thread-a" },
            "model": "gpt-new",
            "modelProvider": "openai"
        }
    });
    let (mut backend, _) = backend([response]);
    backend.model_rebind_target = Some((
        AccountId::new("account-test").unwrap(),
        ModelId::new("gpt-new").unwrap(),
    ));

    let failure = backend
        .rebind_model(session(1), &rebind_source("thread-a", "gpt-old"))
        .unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Session);
}

// 연결 대상 검증은 initialize와 account/read까지만 수행하고 thread/start나 thread/resume을
// 보내지 않아, binding에 필요한 account 검증이 새 backend Session을 만들지 않음을 확인합니다.
#[test]
fn verifies_the_handshake_without_creating_a_thread() {
    let (peer, sent) = FakePeer::new([
        initialize_response(1, "0.146.0"),
        json!({
            "id": 2,
            "result": {
                "account": {
                    "type": "chatgpt",
                    "email": "person@example.test",
                    "planType": "pro"
                }
            }
        }),
    ]);
    let client = AppServerClient::new(peer, Duration::from_secs(1));
    let mut backend = Backend::new_uninitialized(client, "/workspace".into(), false, None);

    backend.verify().unwrap();

    let sent = sent.0.borrow();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[0]["method"], "initialize");
    assert_eq!(sent[1], json!({ "method": "initialized" }));
    assert_eq!(sent[2]["method"], "account/read");
    assert!(sent.iter().all(|message| {
        !matches!(
            message.get("method").and_then(serde_json::Value::as_str),
            Some("thread/start" | "thread/resume")
        )
    }));
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

// 읽기 전용 리뷰는 thread와 turn 양쪽에 승인 금지·읽기 전용 sandbox를 반복하고,
// network 차단과 durable execution profile을 증거에 함께 고정합니다.
#[test]
fn read_only_review_applies_closed_thread_and_turn_policies() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let turn_response = json!({
        "id": 3,
        "result": { "turn": { "id": "turn-a" } }
    });
    let (mut backend, sent) =
        backend_with_profile([thread_start_response(2, "thread-a"), turn_response], true);

    let evidence = backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    let BackendCommandEvidence::BindingOpened(binding) = evidence else {
        panic!("thread/start must return the restricted binding");
    };
    assert_eq!(
        binding.binding_identity().schema(),
        "codex.app-server/thread-binding/v1alpha2"
    );
    assert!(
        binding
            .binding_identity()
            .value()
            .contains("yo.delegated-review-execution/v1alpha1")
    );
    assert!(binding.binding_identity().value().contains("account-test"));

    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("review this"),
        })
        .unwrap();

    let sent = sent.0.borrow();
    assert_eq!(sent[2]["params"]["approvalPolicy"], "never");
    assert_eq!(sent[2]["params"]["sandbox"], "read-only");
    assert_eq!(sent[3]["params"]["approvalPolicy"], "never");
    assert_eq!(
        sent[3]["params"]["sandboxPolicy"],
        json!({ "type": "readOnly", "networkAccess": false })
    );
}

// 제한 Session resume은 같은 v1alpha1 profile만 받아 thread/resume에도 정책을 다시
// 적용하고, 일반 backend로 잘못 재개하면 RPC 전 Session 실패로 닫습니다.
#[test]
fn read_only_resume_restores_policy_and_rejects_profile_downgrade() {
    let (mut restricted, restricted_sent) =
        backend_with_profile([thread_start_response(2, "thread-a")], true);
    restricted
        .resume_binding(session(1), &read_only_resume_binding("thread-a"))
        .unwrap();
    assert_eq!(
        restricted_sent.0.borrow()[2]["params"],
        json!({
            "threadId": "thread-a",
            "approvalPolicy": "never",
            "sandbox": "read-only",
        })
    );

    let (mut standard, standard_sent) = backend([]);
    let failure = standard
        .resume_binding(session(1), &read_only_resume_binding("thread-a"))
        .unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Session);
    assert_eq!(standard_sent.0.borrow().len(), 2);
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

// 검증하지 않은 같은-major Codex minor 버전은 경고를 제공하면서도 initialized까지 보내
// 사용자의 로컬 Codex 업데이트가 Yo Session 시작 자체를 막지 않게 한다.
#[test]
fn unverified_minor_warns_and_completes_initialization() {
    let (peer, sent) = FakePeer::new([initialize_response(1, "0.150.0")]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));

    let initialize = client.initialize().unwrap();

    assert!(
        initialize
            .compatibility_warning
            .as_deref()
            .is_some_and(|warning| warning.contains("0.150.0"))
    );
    assert_eq!(sent.0.borrow().len(), 2);
    assert_eq!(sent.0.borrow()[1], json!({ "method": "initialized" }));
}
