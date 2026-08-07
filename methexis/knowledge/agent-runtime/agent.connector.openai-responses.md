---
schema: methexis.knowledge/v1alpha1
id: agent.connector.openai-responses
kind: decision
owner: agent-runtime
sources:
  - id: agent.connector-001
    revision: sha256:9ab4453251718bec261e65300269fedc7c50e07c23bc7b1e7921f0fa7afbb137
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.credentials.local-account-store
    - agent.model.service-binding
  constrained_by:
    - agent.observability.session-journal
---
# OpenAI Responses model connector

## Statement

The first Model Connector MUST implement the explicit `openai-responses`
protocol over HTTP with SSE streaming. It MUST be provider-neutral and MUST NOT
be named for QwenCloud. Its configuration MUST contain an absolute normalized
HTTPS base URL, the exact protocol, and the effective Model binding. It MUST
reject URL user information, query, and fragment components. Redirect handling
MUST be bounded and MUST accept only a normalized HTTPS target on the exact same
origin. A cross-origin redirect MUST fail without forwarding credentials,
model context, tool schemas, arguments, or results.

The connector MUST append the single path segment `responses` to the normalized
base URL. It MUST NOT append another `v1`, infer a vendor path, probe a second
endpoint, or fall back to Chat Completions. The initial QwenCloud Token Plan
profile therefore resolves:

```text
base_url    = https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1
request_url = https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1/responses
model       = qwen3.8-max
```

The connector MUST send the resolved API key as bearer authorization without
making it observable outside the HTTP client. A request MUST identify the wire
model, input items, declared function tools, tool choice, streaming mode, and
only model options supported by the selected binding. The initial Qwen binding
MAY carry `reasoning.effort`; it MUST NOT enable provider session cache,
`previous_response_id`, or a provider Conversation as the authority for Yo
continuation.

The SSE decoder MUST preserve exact text bytes and function-call `call_id`,
name, and JSON argument bytes while correlating deltas to their output item.
It MUST report response completion, incomplete or failed termination, usage,
and reasoning-token counts when present. Unknown JSON fields MAY be ignored.
An unknown output item, malformed correlation, duplicate terminal event, text
after termination, invalid UTF-8, or stream end without a terminal response
MUST be a typed protocol failure rather than a completed Turn.

Every binding profile MUST set finite connect, response-header, stream-idle,
and total request deadlines plus maximum error-body bytes, SSE-event bytes,
SSE-event count, output-item count, cumulative response-text bytes, and
cumulative function-argument bytes. The connector MUST enforce those bounds
while reading rather than after buffering an unbounded value. Deadline expiry,
oversized HTTP or SSE data, event-count or cumulative response overflow, and
cancellation during decoding MUST terminate with a typed failure and MUST
follow the same partial-response retry prohibition below.

An HTTP status that explicitly reports throttling or temporary service failure
MAY be retried within a bounded policy before any response item is admitted.
Connection ambiguity after request transmission and any failure after the first
response item MUST NOT be retried automatically. Every retry attempt MUST keep
its own request correlation; the connector MUST never repeat a tool result or
hide a partial stream behind a replacement response.

## Rationale

An exact protocol and URL join rule make OpenAI compatibility testable instead
of vendor inference. Keeping provider session state out of continuation lets
Yo's durable semantic Journal remain authoritative while still supporting
QwenCloud through the generic connector.
