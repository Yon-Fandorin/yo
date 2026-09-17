use super::{super::support::*, *};

// Codex token usage의 필수 cachedInputTokens가 음수이면 영수증을 생성하지 않고
// Protocol 실패로 닫아 provider가 보고하지 않은 유효값을 추측하지 않습니다.
#[test]
fn rejects_malformed_codex_token_usage() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        thread_start_response(2, "thread-a"),
        json!({ "id": 3, "result": { "turn": { "id": "turn-a" } } }),
        json!({
            "method": "thread/tokenUsage/updated",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "tokenUsage": {
                    "last": {
                        "inputTokens": 10,
                        "cachedInputTokens": -1,
                        "outputTokens": 2,
                        "reasoningOutputTokens": 1,
                        "totalTokens": 12
                    },
                    "total": {
                        "inputTokens": 10,
                        "cachedInputTokens": 0,
                        "outputTokens": 2,
                        "reasoningOutputTokens": 1,
                        "totalTokens": 12
                    }
                }
            }
        }),
    ];
    let (mut backend, _) = backend(messages);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("inspect"),
        })
        .unwrap();

    let failure = backend.poll_event().unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("cached input tokens"));
}
