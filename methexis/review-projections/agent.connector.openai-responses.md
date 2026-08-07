---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.connector.openai-responses
revision: sha256:517d11c14064f16e5a9037e640672b1255b9a7f7b01f3d978111019605f94d54
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:41b3b531dd01f2e311c34847498b4312ff3cdb024243461508397f6b1ea2e57e
---
# Korean Review Projection

## Translation

# OpenAI Responses model connector

## 계약

첫 Model Connector는 HTTP와 SSE streaming을 사용하는 명시적인 openai-responses protocol을 구현합니다. Provider-neutral이어야 하며 QwenCloud 이름을 붙이지 않습니다. 설정은 absolute normalized HTTPS base URL, exact protocol, effective Model binding을 포함합니다. URL user information, query, fragment를 거부합니다. Redirect는 횟수를 제한하고 동일한 normalized HTTPS origin의 target만 허용합니다. Cross-origin redirect는 credential뿐 아니라 model context, tool schema, argument, result도 보내지 않고 실패합니다.

Connector는 normalized base URL에 responses path segment 하나만 붙입니다. v1을 다시 붙이거나 vendor path를 추론하거나 다른 endpoint를 probe하거나 Chat Completions로 fallback하지 않습니다. QwenCloud Token Plan은 base URL https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1, request URL https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1/responses, model qwen3.8-max로 resolve됩니다.

Resolve된 API key는 HTTP client 밖으로 관찰되지 않는 bearer authorization으로 보냅니다. Request는 wire model, input items, function tools, tool choice, streaming mode, selected binding이 지원하는 model option만 포함합니다. 초기 Qwen binding은 reasoning.effort를 사용할 수 있지만 provider session cache, previous_response_id, provider Conversation을 Yo continuation authority로 사용하지 않습니다.

SSE decoder는 output item에 delta를 correlate하며 정확한 text bytes와 function call의 call_id, name, JSON argument bytes를 보존합니다. Response completion, incomplete 또는 failed termination, usage, 존재할 때 reasoning-token count를 보고합니다. Unknown JSON field는 무시할 수 있지만 unknown output item, malformed correlation, duplicate terminal event, termination 뒤 text, invalid UTF-8, terminal response 없는 stream end는 completed Turn이 아니라 typed protocol failure입니다.

모든 binding profile은 finite connect, response-header, stream-idle, total request deadline과 maximum error-body bytes, SSE-event bytes, SSE-event count, output-item count, cumulative response-text bytes, cumulative function-argument bytes를 정합니다. Connector는 unbounded buffering 뒤가 아니라 읽는 동안 이 bound를 적용합니다. Deadline expiry, oversized HTTP 또는 SSE data, event-count 또는 cumulative response overflow, decoding 중 cancellation은 typed failure이며 partial-response retry 금지를 그대로 적용합니다.

Throttling이나 temporary service failure를 명시적으로 보고한 HTTP status는 response item을 하나도 admit하기 전에 bounded retry할 수 있습니다. Request 전송 뒤의 connection ambiguity와 첫 response item 뒤의 failure는 자동 retry하지 않습니다. 각 retry attempt는 자기 request correlation을 유지하며 tool result를 반복하거나 partial stream을 replacement response로 숨기지 않습니다.

## 이유

Exact protocol과 URL join rule은 OpenAI compatibility를 vendor 추론이 아니라 test 가능한 계약으로 만듭니다. Same-origin redirect와 streaming deadline 및 cumulative resource bound는 credential과 semantic payload 유출, stalled 또는 unbounded response를 막습니다. Provider session state를 continuation에서 제외하면 generic connector로 QwenCloud를 지원하면서 Yo durable semantic Journal을 authority로 유지할 수 있습니다.
