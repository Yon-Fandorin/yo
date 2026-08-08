---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.connector.openai-responses
revision: sha256:811c1d8c896b7976d32c03c3dd131f95e99ad328807b072c23ca6518a671ce19
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:b5fa00b8b731dc98d78ada61f97b82f918e37abdf7164721ff0ca74a7967d210
---
# Korean Review Projection

## Translation

# OpenAI Responses model connector

## 규칙

첫 Model Connector는 HTTPS와 SSE streaming 위에서 명시적인 `openai-responses` API dialect를 구현해야 합니다. Provider-neutral해야 하며 OpenRouter, QwenCloud 또는 특정 Model family 이름을 사용하면 안 됩니다. OpenRouter와 QwenCloud binding은 같은 connector 구현을 사용합니다. 입력에는 절대 경로로 정규화된 HTTPS base URL, 정확한 dialect, effective Model binding, 정확한 Provider와 Account 조합에 대해 미리 해석된 opaque credential 하나가 포함되어야 합니다.

Connector는 URL user information, query, fragment를 거절합니다. Redirect는 횟수를 제한하고 완전히 같은 origin의 정규화된 HTTPS target만 허용합니다. Cross-origin redirect는 credential, model context, tool schema, arguments, results를 전달하지 않고 실패해야 합니다. Connector는 정규화된 base URL에 `responses` path segment 하나만 추가합니다. 추가 `v1`, vendor path 추론, 다른 endpoint probe, Chat Completions fallback은 금지합니다. 구체적인 Provider endpoint, Model ID, tokenizer profile은 connector identity가 아니라 catalog 설정과 conformance evidence에 속합니다.

해석된 API key는 HTTP client 밖에서 관찰되지 않도록 bearer authorization으로 보냅니다. Request는 wire Model, input item, 선언된 function tool, tool choice, streaming mode, output-token cap, 선택된 binding이 지원하는 model option만 포함해야 합니다. Provider session cache, `previous_response_id`, provider Conversation을 Yo continuation authority로 사용하면 안 됩니다.

SSE decoder는 text byte와 function-call `call_id`, name, JSON argument byte를 정확히 보존하면서 output item과 delta를 연결합니다. Response completion, incomplete/failed terminal, usage, reasoning-token count가 있으면 보고해야 합니다. 알 수 없는 JSON field는 무시할 수 있습니다. 유효한 `response.completed`, `response.incomplete`, `response.failed` event만 semantic terminal입니다. `data` field가 없는 SSE event는 terminal 전후의 transport framing으로 무시할 수 있습니다. 유효한 semantic terminal 뒤에는 declared event name이 없는 정확한 `data: [DONE]` payload 하나를 제외한 모든 `data` field 포함 SSE event가 실패해야 합니다. 이 sentinel은 마지막 data payload여야 하고 terminal을 대신하거나 반복할 수 없으며, 그 뒤에는 data 없는 framing과 stream 종료만 허용합니다. `[DONE]`이 terminal 전에 나오거나 event name을 선언하면 실패해야 합니다. 알 수 없는 output item, 잘못된 correlation, 중복 terminal, 종료 뒤 그 밖의 모든 data, invalid UTF-8, terminal 없는 stream 종료는 completed Turn이 아니라 typed protocol failure가 되어야 합니다.

각 binding profile은 유한한 connect, response-header, stream-idle, total request deadline과 error-body bytes, SSE-event bytes/count, output-item count, 누적 response-text bytes, 누적 function-argument bytes 최대값을 설정합니다. Connector는 무제한 buffering 뒤가 아니라 읽는 동안 제한을 적용해야 합니다. Deadline expiry, oversized HTTP/SSE data, event-count 또는 누적 response overflow, decoding 중 cancellation은 typed failure로 종료하며 같은 partial-response retry 금지 규칙을 따라야 합니다.

명시적인 throttling 또는 temporary service failure HTTP status는 response item이 하나도 admitted되지 않은 경우에만 제한된 정책으로 retry할 수 있습니다. Request 전송 뒤 connection ambiguity와 첫 response item 뒤 모든 failure는 자동 retry하면 안 됩니다. 각 retry attempt는 자체 request correlation을 유지해야 하며 tool result를 반복하거나 partial stream을 대체 response 뒤에 숨기면 안 됩니다.

## 이유

정확한 dialect와 URL 결합 규칙은 vendor 추론 대신 OpenAI compatibility를 검증 가능하게 합니다. Provider-neutral transport와 decoding을 통해 OpenRouter와 QwenCloud가 하나의 backend architecture를 공유합니다. Semantic terminal을 필수로 유지하면서 실제 관찰된 post-terminal transport sentinel만 좁게 허용하면 fail-closed completion을 보존하면서 compatible SSE stream을 거절하지 않습니다.
