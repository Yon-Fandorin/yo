---
schema: methexis.knowledge/v1alpha1
id: agent.connector.openai-responses
kind: decision
owner: agent-runtime
sources:
  - id: agent.connector-001
    revision: sha256:c9f05e8d0f10bfd7ba40755c6b797ceb5c9005412b499a88a43500d7d6b96775
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

The first Model Connector MUST implement the explicit `openai-responses` API dialect over HTTPS with SSE streaming. It MUST be provider-neutral and MUST NOT be named for OpenRouter, QwenCloud, or any Model family. OpenRouter and QwenCloud bindings MUST use the same connector implementation. Its input MUST contain an absolute normalized HTTPS base URL, the exact dialect, the effective Model binding, and one opaque credential already resolved for the exact Provider-and-Account pair.

The connector MUST reject URL user information, query, and fragment components. Redirect handling MUST be bounded and MUST accept only a normalized HTTPS target on the exact same origin. A cross-origin redirect MUST fail without forwarding credentials, model context, tool schemas, arguments, or results. The connector MUST append the single path segment `responses` to the normalized base URL. It MUST NOT append another `v1`, infer a vendor path, probe a second endpoint, or fall back to Chat Completions. Concrete Provider endpoints, Model IDs, and tokenizer profiles belong to catalog configuration and conformance evidence rather than this connector identity.

The connector MUST send the resolved API key as bearer authorization without making it observable outside the HTTP client. A request MUST identify the wire Model, input items, declared function tools, tool choice, streaming mode, output-token cap, and only model options supported by the selected binding. It MUST NOT enable provider session cache, `previous_response_id`, or a provider Conversation as the authority for Yo continuation.

The SSE decoder MUST preserve exact text bytes and function-call `call_id`, name, and JSON argument bytes while correlating deltas to their output item. It MUST report response completion, incomplete or failed termination, usage, and reasoning-token counts when present. Unknown JSON fields MAY be ignored. A valid `response.completed`, `response.incomplete`, or `response.failed` event is the only semantic terminal. An SSE event with no `data` field MAY be ignored as transport framing before or after termination. After a valid semantic terminal, every SSE event containing a `data` field MUST fail except at most one exact `data: [DONE]` payload with no declared event name. That sentinel MUST be the final data payload, MUST NOT replace a terminal or repeat, and MAY be followed only by non-data framing and stream end. It MUST fail when `[DONE]` appears before a terminal or declares an event name. An unknown output item, malformed correlation, duplicate terminal event, any other data after termination, invalid UTF-8, or stream end without a terminal response MUST be a typed protocol failure rather than a completed Turn.

Every binding profile MUST set finite connect, response-header, stream-idle, and total request deadlines plus maximum error-body bytes, SSE-event bytes, SSE-event count, output-item count, cumulative response-text bytes, and cumulative function-argument bytes. The connector MUST enforce those bounds while reading rather than after buffering an unbounded value. Deadline expiry, oversized HTTP or SSE data, event-count or cumulative response overflow, and cancellation during decoding MUST terminate with a typed failure and MUST follow the same partial-response retry prohibition below.

An HTTP status that explicitly reports throttling or temporary service failure MAY be retried within a bounded policy before any response item is admitted. Connection ambiguity after request transmission and any failure after the first response item MUST NOT be retried automatically. Every retry attempt MUST keep its own request correlation; the connector MUST never repeat a tool result or hide a partial stream behind a replacement response.

## Rationale

An exact dialect and URL join rule make OpenAI compatibility testable instead of vendor inference. Provider-neutral transport and decoding let OpenRouter and QwenCloud share one backend architecture. Requiring a semantic terminal while narrowly tolerating an observed post-terminal transport sentinel preserves fail-closed completion without rejecting a compatible SSE stream.
