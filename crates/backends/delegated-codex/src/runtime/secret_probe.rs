//! Codex 동적 도구와 Yo 숨김 입력 사이의 위임형 비밀 상호작용입니다.

use serde_json::{Map, Value, json};
use yo_backend::transport::JsonMessagePeer;
use yo_core::{BackendFailure, BackendFailureKind};

use super::state::{Backend, DelegatedSecretTool, InputQuestions};
use crate::protocol;

pub(super) const TOOL_NAME: &str = "yo_secret_entry_probe";
pub(super) const DELIVERY_TOOL_NAME: &str = "yo_request_secret_input";

pub(super) fn wire_version_supported(user_agent: &str) -> bool {
    user_agent
        .split_whitespace()
        .next()
        .and_then(|part| part.strip_prefix("yo/"))
        == Some("0.155.1")
}

pub(super) fn is_secret_tool_name(name: &str) -> bool {
    matches!(name, TOOL_NAME | DELIVERY_TOOL_NAME)
}

pub(super) fn tool_spec(tool: DelegatedSecretTool) -> Value {
    match tool {
        DelegatedSecretTool::Probe => json!({
            "type": "function",
            "name": TOOL_NAME,
            "description": "Diagnostic only: ask the user to enter a SAMPLE secret in Yo's hidden editor. Yo discards the value and returns only a fixed status. Never request a real password, token, or credential with this tool; the model cannot use the entered value.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        }),
        DelegatedSecretTool::Deliver => json!({
            "type": "function",
            "name": DELIVERY_TOOL_NAME,
            "description": "Ask the user for one secret value needed for the current task. Yo sends the answer once to Codex and its selected model. They may retain or reuse it. Arguments must contain only the public request.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Short public title for the secret request."
                    },
                    "question": {
                        "type": "string",
                        "description": "Public question shown before secret entry."
                    },
                    "purpose": {
                        "type": "string",
                        "description": "Public reason the current task needs the secret."
                    }
                },
                "required": ["title", "question", "purpose"],
                "additionalProperties": false
            }
        }),
    }
}

pub(super) fn parse_request<P: JsonMessagePeer>(
    backend: &mut Backend<P>,
    params: &Value,
) -> Result<InputQuestions, BackendFailure> {
    let tool = backend.secret_tool.ok_or_else(|| {
        BackendFailure::new(
            BackendFailureKind::Unsupported,
            "Codex delegated secret interaction is not enabled for this Session",
        )
    })?;
    if backend.read_only_review {
        return Err(BackendFailure::new(
            BackendFailureKind::Unsupported,
            "Codex delegated secret interaction is unavailable for read-only review",
        ));
    }
    if !wire_version_supported(backend.backend_version.as_deref().unwrap_or_default()) {
        return Err(BackendFailure::new(
            BackendFailureKind::Unsupported,
            "delegated secret interaction requires the reviewed Codex 0.155.1 dynamic-tool wire",
        ));
    }
    let expected_name = match tool {
        DelegatedSecretTool::Probe => TOOL_NAME,
        DelegatedSecretTool::Deliver => DELIVERY_TOOL_NAME,
    };
    if params
        .get("namespace")
        .is_some_and(|value| !value.is_null())
        || protocol::string_at(params, &["tool"])? != expected_name
    {
        return Err(protocol::protocol_failure("unknown Codex dynamic tool"));
    }
    let call_id = protocol::string_at(params, &["callId"])?;
    let wire_turn = protocol::string_at(params, &["turnId"])?;
    if call_id.is_empty()
        || !backend.items.get(call_id).is_some_and(|item| {
            item.dynamic_tool_call.as_ref().is_some_and(|call| {
                call.tool == expected_name && call.arguments == params["arguments"]
            }) && backend.wire_turns.get(wire_turn).is_some_and(|turn| {
                item.activity.turn() == turn.turn && !turn.finished && !turn.interrupted
            })
        })
    {
        return Err(protocol::protocol_failure(
            "Codex dynamic tool call does not match an active item",
        ));
    }
    let args = params
        .get("arguments")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            protocol::protocol_failure("Codex dynamic tool arguments must be an object")
        })?;
    let mut parsed = match tool {
        DelegatedSecretTool::Probe => parse_probe(args)?,
        DelegatedSecretTool::Deliver => parse_delivery(args)?,
    };
    parsed.secret_tool = Some(tool);
    // 입력 화면을 게시하기 전에 시작된 호출을 소비합니다. 이후 실패는 같은 호출이
    // 또 다른 입력 화면을 열지 못하도록 닫힌 상태로 유지합니다.
    backend
        .items
        .get_mut(call_id)
        .expect("validated active dynamic tool item")
        .dynamic_tool_call
        .take();
    Ok(parsed)
}

fn parse_probe(args: &Map<String, Value>) -> Result<InputQuestions, BackendFailure> {
    if !args.is_empty() {
        return Err(protocol::protocol_failure(
            "Codex secret-entry probe takes no arguments",
        ));
    }
    InputQuestions::parse(&json!({"questions": [{
        "id": "sample-secret",
        "header": "Sample input test",
        "question": "Enter a made-up sample value only. Never enter a real password, token, or credential. Yo will discard the sample without sending it to Codex or the model.",
        "isSecret": true,
        "options": []
    }]}))
}

fn parse_delivery(args: &Map<String, Value>) -> Result<InputQuestions, BackendFailure> {
    if args.len() != 3
        || !args
            .keys()
            .all(|key| matches!(key.as_str(), "title" | "question" | "purpose"))
    {
        return Err(protocol::protocol_failure(
            "Codex delegated secret request must contain only title, question, and purpose",
        ));
    }
    let title = public_argument(args, "title", 80, false)?;
    let question = public_argument(args, "question", 4096, true)?;
    let purpose = public_argument(args, "purpose", 4096, true)?;
    InputQuestions::parse(&json!({"questions": [{
        "id": "delegated-secret",
        "header": title,
        "question": format!("{question}\n\nPurpose: {purpose}"),
        "isSecret": true,
        "options": []
    }]}))
}

fn public_argument<'a>(
    args: &'a Map<String, Value>,
    name: &str,
    max_bytes: usize,
    multiline: bool,
) -> Result<&'a str, BackendFailure> {
    let value = args.get(name).and_then(Value::as_str).ok_or_else(|| {
        protocol::protocol_failure(format!(
            "Codex delegated secret request {name} must be a string"
        ))
    })?;
    let invalid_control = value.chars().any(|character| {
        character.is_control() && !(multiline && matches!(character, '\t' | '\n' | '\r'))
    });
    if value.is_empty() || value.len() > max_bytes || invalid_control {
        return Err(protocol::protocol_failure(format!(
            "Codex delegated secret request {name} is invalid or exceeds its byte limit"
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 공개 요청 인자는 정확한 필드와 UTF-8 byte 경계를 지키며 허용된 줄바꿈만 받습니다.
    #[test]
    fn delivery_request_validates_public_argument_shape_and_bounds() {
        let valid = Map::from_iter([
            ("title".to_owned(), json!("한".repeat(26))),
            ("question".to_owned(), json!("첫 줄\n둘째 줄\t설명")),
            ("purpose".to_owned(), json!("현재 요청 인증")),
        ]);
        let parsed = parse_delivery(&valid).unwrap();
        assert_eq!(parsed.questions.len(), 1);
        assert!(parsed.questions[0].question.contains("둘째 줄"));

        for invalid in [
            Map::from_iter([
                ("title".to_owned(), json!("x".repeat(81))),
                ("question".to_owned(), json!("질문")),
                ("purpose".to_owned(), json!("목적")),
            ]),
            Map::from_iter([
                ("title".to_owned(), json!("제목\n노출")),
                ("question".to_owned(), json!("질문")),
                ("purpose".to_owned(), json!("목적")),
            ]),
            Map::from_iter([
                ("title".to_owned(), json!("제목")),
                ("question".to_owned(), json!("질문\u{0}")),
                ("purpose".to_owned(), json!("목적")),
            ]),
            Map::from_iter([
                ("title".to_owned(), json!("제목")),
                ("question".to_owned(), json!("질문")),
                ("purpose".to_owned(), json!("목적")),
                ("secret".to_owned(), json!("공개 인자에 둘 수 없음")),
            ]),
        ] {
            assert!(parse_delivery(&invalid).is_err());
        }
    }
}
