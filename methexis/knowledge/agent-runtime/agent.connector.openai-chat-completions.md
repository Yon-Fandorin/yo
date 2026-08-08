---
schema: methexis.knowledge/v1alpha1
id: agent.connector.openai-chat-completions
kind: decision
owner: agent-runtime
sources:
  - id: agent.connector-002
    revision: sha256:acf964793f2e199cf1458404b8560c6c11f46b1ded9aece6ff83eecf167604be
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.credentials.local-account-store
    - agent.model.service-binding
  constrained_by:
    - agent.observability.session-journal
---
# OpenAI Chat Completions model connector

## Statement

The Model Connector for the explicit `openai-chat-completions` API dialect MUST use HTTPS with SSE streaming. It MUST be provider-neutral and MUST NOT be named for OpenRouter, QwenCloud, DeepSeek, or any Model family. Its input MUST contain an absolute normalized HTTPS base URL, the exact dialect, the effective Model binding, and one opaque credential already resolved for the exact Provider-and-Account pair. The connector MUST start only for the dialect-derived `openai-chat-completions` connector-and-dialect pair.

The connector MUST reject URL user information, query, and fragment components. Redirect handling MUST be bounded and MUST accept only a normalized HTTPS target on the exact same origin. A cross-origin redirect MUST fail without forwarding credentials, model context, tool schemas, arguments, or results. The connector MUST append the two path segments `chat` and `completions` to the normalized base URL. It MUST NOT append another `v1`, infer a vendor path, probe a second endpoint, or fall back to Responses. Concrete Provider endpoints, Model IDs, tokenizer profiles, model defaults, and optional Provider parameters belong to catalog configuration and conformance evidence rather than this connector identity.

The connector MUST send the resolved API key as bearer authorization without making it observable outside the HTTP client. A request MUST identify the wire Model, an ordered `messages` replay, declared function `tools`, automatic tool choice, streaming mode, `stream_options.include_usage`, and the configured output-token cap as `max_tokens`. The first profile MUST NOT inject an undeclared Provider-specific option. Replay MUST encode system and visible text messages with their exact roles and bytes. A prior assistant round MUST be encoded as one assistant message preserving its exact visible `content`, visible `refusal`, and correlated `tool_calls` as independent fields, including when content or refusal accompanied tool calls. Each admitted tool result MUST follow as a `tool` message carrying the exact `tool_call_id`. The connector MUST NOT send provider conversation state or hidden reasoning content as continuation authority.

The SSE decoder MUST accept only one correlated choice with index zero. It MUST preserve exact `delta.content` bytes; MUST expose exact `delta.refusal` bytes as a visible refusal observation; MAY expose `delta.reasoning_content` as a reasoning observation without adding it to exact replay; and MUST correlate indexed `delta.tool_calls` fragments while preserving each call ID, function name, and exact accumulated JSON argument bytes. Content, refusal, and tool-call deltas are independent optional fields and MAY coexist in one assistant round. A refusal completed with `stop` MUST be displayed and committed as a completed visible assistant response rather than classified as a transport or protocol failure, and its exact visible bytes MUST remain in replay. Tool-call indexes MUST first appear contiguously in ascending order; later fragments for admitted indexes MAY interleave. An initial role-only delta MAY be ignored. Multiple choices, a changed response ID, a duplicate or missing tool-call ID, inconsistent function names, a non-contiguous first index or fragment for an unintroduced index, or a malformed delta MUST be a typed protocol failure.

Exactly one choice finish reason MUST precede termination. `stop` denotes a completed final-assistant round and MUST fail as contradictory when tool calls were accumulated. `tool_calls` denotes a completed tool-call round and MUST fail as contradictory when no tool call was accumulated; any accompanying content or refusal remains part of the same assistant message and exact replay. `length` denotes a typed incomplete failure: the backend MUST fail the Turn, MUST NOT commit the partial assistant message or tool calls to replay, and MUST NOT publish a Continuation Anchor covering them. `content_filter` denotes a typed failed response with the same replay and Anchor exclusion. An unknown, duplicate, or contradictory finish reason MUST fail closed.

An absent or null `usage` field on an ordinary choice chunk is transport metadata and MUST be ignored. Because the request asks for streaming usage, exactly one non-null final usage record in a chunk with an empty `choices` array MUST follow the finished choice and MUST report non-negative `prompt_tokens`, `completion_tokens`, and `total_tokens`; a non-negative `completion_tokens_details.reasoning_tokens` MAY also be reported. A non-null usage record before finish, duplicate non-null usage, a non-empty choice array on the final usage record, negative values, or totals inconsistent with prompt and completion tokens MUST be a typed protocol failure.

The exact un-named `data: [DONE]` sentinel MUST be the final data payload after both the finished choice and usage record. It is the Chat Completions stream terminal and MUST NOT be treated as JSON. It MUST NOT be missing, repeated, or carry an SSE event name. SSE comments and events with no data field MAY be ignored as transport framing before `[DONE]`; after `[DONE]`, only non-data framing and stream end are permitted. Any other data after `[DONE]`, invalid UTF-8, or stream end without the complete finish, usage, and sentinel sequence MUST be a typed protocol failure rather than a completed Turn.

Every binding profile MUST set finite connect, response-header, stream-idle, and total request deadlines plus maximum error-body bytes, SSE-event bytes, SSE-event count, tool-call count, cumulative response-content bytes, cumulative refusal bytes, cumulative reasoning bytes, and cumulative function-argument bytes. The connector MUST enforce those bounds while reading rather than after buffering an unbounded value. Deadline expiry, oversized HTTP or SSE data, event-count or cumulative response overflow, and cancellation during decoding MUST terminate with a typed failure and MUST follow the same partial-response retry prohibition below.

An HTTP status that explicitly reports throttling or temporary service failure MAY be retried within a bounded policy before any response item is admitted. Connection ambiguity after request transmission and any failure after the first response item MUST NOT be retried automatically. Every retry attempt MUST keep its own request correlation; the connector MUST never repeat a tool result or hide a partial stream behind a replacement response.

## Rationale

Chat Completions and Responses share transport concerns but have different message, tool-call, usage, and terminal grammars. A separate exact dialect and provider-neutral connector preserve those differences while allowing the same Yo-managed semantic model loop to use either one. Treating refusal as visible model output preserves the user's conversation rather than misreporting a normal model decision as infrastructure failure. Requiring the finish, usage, and `[DONE]` sequence while excluding `length` and failed responses from replay prevents a truncated stream from becoming durable model history.
