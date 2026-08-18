---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.connector.openai-responses
revision: sha256:864ad59d661e3fcb3dbf393fbb669ba439a9372dfa24fac3f361c4866ec28019
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:3dc1f4badaa164347f0d4801c55179381f65bb2060ddc41c7a21c82e30d13b87
---
# Korean Review Projection

## Translation

# OpenAI Responses 모델 connector

## 규칙

첫 Model Connector는 HTTPS와 SSE streaming 위에서 명시적인 `openai-responses` API dialect를 구현해야 합니다. Provider-neutral해야 하며 OpenRouter, QwenCloud 또는 특정 Model family 이름을 사용하면 안 됩니다. OpenRouter와 QwenCloud binding은 같은 connector 구현을 사용해야 합니다. 입력에는 절대 경로로 정규화된 HTTPS base URL, 정확한 dialect, effective Model binding, 정확한 Provider와 Account 조합에 대해 미리 해석된 opaque credential 하나가 포함되어야 합니다.

Connector는 URL user information, query, fragment를 거절해야 합니다. Redirect는 횟수를 제한하고 완전히 같은 origin의 정규화된 HTTPS target만 허용해야 합니다. Cross-origin redirect는 credential, model context, tool schema, argument, result를 전달하지 않고 실패해야 합니다. Connector는 정규화된 base URL에 `responses` path segment 하나만 추가해야 합니다. 추가 `v1`, vendor path 추론, 다른 endpoint probe, Chat Completions fallback은 금지합니다. 구체적인 Provider endpoint, Model ID, tokenizer profile은 connector identity가 아니라 catalog 설정과 conformance evidence에 속합니다.

해석된 API key는 HTTP client 밖에서 관찰되지 않도록 bearer authorization으로 보내야 합니다. 모든 request는 wire Model, input item, streaming mode, 선택한 binding이 지원하는 model option, 허용된 request-local tool exposure를 식별해야 합니다. Effective profile에 알려진 hard `max_output_tokens`가 있으면 그 이하의 양수 request-local cap 하나를 `max_output_tokens`로 보내야 합니다. Yo가 maximum을 몰라 profile에 필드가 없으면 숫자를 추측하지 않고 request의 `max_output_tokens`도 생략해야 합니다. Enabled exposure는 effective `local-tools/v1`에서만 유효하며 frozen registry의 선언된 function tool과 정확한 tool-choice field를 포함해야 합니다. Disabled exposure는 tool definition과 tool-selection field를 모두 생략해야 합니다. Effective `no-tools/v1`과 모든 connection-verification request는 disabled exposure를 요구하며, `local-tools/v1` binding의 verification request도 durable policy는 바꾸지 않습니다. 과거 function-call 및 function-call-output input item은 정확한 semantic replay로 남고 current tool registry를 노출하지 않습니다. 다른 tool policy 또는 policy와 exposure 조합은 transport 전에 실패해야 합니다. Provider session cache, `previous_response_id`, provider Conversation을 Yo continuation authority로 사용하면 안 됩니다.

SSE decoder는 text byte와 function-call `call_id`, name, JSON argument byte를 정확히 보존하면서 output item과 delta를 연결해야 합니다. Response completion, incomplete 또는 failed termination, usage, reasoning-token count가 있으면 보고해야 합니다. 알 수 없는 JSON field는 무시할 수 있습니다. 유효한 `response.completed`, `response.incomplete`, `response.failed` event만 semantic terminal입니다. `data` field가 없는 SSE event는 terminal 전후의 transport framing으로 무시할 수 있습니다. 유효한 semantic terminal 뒤에는 declared event name이 없는 정확한 `data: [DONE]` payload 하나를 제외한 모든 `data` field 포함 SSE event가 실패해야 합니다. 이 sentinel은 마지막 data payload여야 하고 terminal을 대신하거나 반복할 수 없으며, 그 뒤에는 data 없는 framing과 stream 종료만 허용합니다. `[DONE]`이 terminal 전에 나오거나 event name을 선언하면 실패해야 합니다. 알 수 없는 output item, 잘못된 correlation, 중복 terminal, 종료 뒤 그 밖의 모든 data, invalid UTF-8, terminal 없는 stream 종료는 completed Turn이 아니라 typed protocol failure가 되어야 합니다.

Connector transport policy는 유한한 connect, response-header, successful-stream-inactivity, error-body-inactivity, internal event-delivery deadline을 설정해야 합니다. 첫 runtime policy는 connect 30초, response header 5분, successful-stream inactivity 5분, error-body inactivity 30초, internal event delivery 또는 backpressure 5분을 사용해야 합니다. 각 phase clock은 다음처럼 동작해야 합니다. 새 connection establishment가 시작될 때마다 connect clock이 시작되고 해당 HTTPS connection이 사용 가능해지면 끝나며, 재사용된 connection에는 새 connect phase가 없습니다. 각 HTTP attempt가 dispatch될 때 response-header clock이 시작되고 connection establishment를 포함해 complete response header가 accept될 때 끝나며 절대 reset되지 않습니다. 성공 header 뒤에는 첫 body chunk 전에 successful-stream inactivity가 시작되고, semantic decoding 전에 들어오는 heartbeat, comment, partial SSE framing byte를 포함한 non-empty raw HTTP body chunk마다 reset됩니다. Transport progress를 증명하기 위해 완성된 SSE event를 요구하면 안 됩니다. Non-success header 뒤에는 첫 error-body chunk 전에 error-body inactivity가 시작되고, retention, truncation 또는 decoding 전에 들어오는 non-empty raw HTTP body chunk마다 reset됩니다. Empty chunk는 어느 inactivity clock도 reset하면 안 됩니다. 각 connector-neutral observation은 handoff할 준비가 된 순간 새로운 별도 absolute event-delivery wait를 시작하고 downstream consumer가 이를 accept할 때만 끝내야 합니다. Network input, 이전 delivery, partial downstream progress는 이 wait를 reset하면 안 되며 다음 observation은 새 wait를 받습니다. 따라서 network input이 active여도 event delivery는 별도로 제한되어야 합니다. 만료는 실패한 phase를 식별해야 합니다.

Absolute model-request deadline은 effective binding이나 connector identity가 아니라 Yo-managed agent policy가 소유합니다. 이 deadline은 선택 사항이며 기본값은 없음이어야 합니다. 설정되면 logical model request 하나에 대해 한 번 시작하고 제한된 connector-internal retry attempt를 포함하며, 수신 byte, decoded event, retry로 reset되면 안 됩니다. 별도로 admit된 이후 model request는 새 deadline을 받습니다. Runtime deadline policy 변경은 binding epoch를 열면 안 됩니다. Agent가 아닌 connection-verification caller는 명시적인 유한 absolute deadline을 제공해야 하며, 첫 verification policy는 위 transport deadline과 함께 10분을 사용해야 합니다. Absolute deadline이 없어도 cancellation은 모든 wait를 중단해야 합니다.

Transport policy는 maximum error-body bytes, SSE-event bytes, SSE-event count, output-item count, 누적 response-text bytes, 누적 function-argument bytes도 설정해야 합니다. Connector는 제한 없는 값을 buffer한 뒤가 아니라 읽는 동안 이 bound를 적용해야 합니다. Deadline 만료, 너무 큰 HTTP 또는 SSE data, event count나 누적 response 초과, decoding 중 cancellation은 typed failure로 종료하고 아래의 같은 partial-response retry 금지를 따라야 합니다.

명시적인 throttling 또는 temporary service failure HTTP status는 response item을 하나도 admit하지 않은 경우 제한된 정책 안에서 retry할 수 있습니다. Request 전송 뒤 connection ambiguity와 첫 response item 이후의 모든 failure는 자동 retry하면 안 됩니다. 각 retry attempt는 자체 request correlation을 유지해야 하며 connector가 tool result를 반복하거나 partial stream을 replacement response 뒤에 숨기면 안 됩니다.

## 이유

정확한 dialect와 URL 결합 규칙은 vendor 추론 대신 OpenAI compatibility를 검증 가능하게 합니다. Provider-neutral transport와 decoding을 통해 OpenRouter와 QwenCloud가 하나의 backend architecture를 공유합니다. Transport progress와 선택적인 agent-owned work budget을 분리하면 건강하게 오래 실행되는 stream이 connector의 보편적인 wall-clock cap 때문에 실패하지 않으면서도 유한한 inactivity, delivery, data, cancellation bound로 멈춘 작업을 탐지할 수 있습니다. Semantic terminal을 필수로 유지하면서 실제 관찰된 post-terminal transport sentinel만 좁게 허용하면 fail-closed completion을 보존하면서 compatible SSE stream을 거절하지 않습니다.
