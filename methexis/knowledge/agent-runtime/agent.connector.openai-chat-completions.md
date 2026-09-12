---
schema: methexis.knowledge/v1alpha1
id: agent.connector.openai-chat-completions
kind: decision
owner: agent-runtime
sources:
  - id: agent.connector-002
    revision: sha256:d0b1a5b4bd1c43b88e6d626f9614532d5eac3948cc64a86b5ad0797af4d71cbd
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

The connector MUST send the resolved API key as bearer authorization without making it observable outside the HTTP client. Every request MUST identify the wire Model, an ordered `messages` replay, streaming mode, `stream_options.include_usage`, and the admitted request-local tool exposure. When the effective profile carries a known hard `max_output_tokens`, the request MUST carry one positive request-local cap no greater than that maximum as `max_tokens`; when the profile omits the maximum because Yo does not know it, the request MUST omit `max_tokens` rather than inventing a number. Enabled exposure is valid only with effective `local-tools/v1` and MUST include the frozen registry's declared function `tools` and automatic tool choice. Disabled exposure MUST omit both tool definitions and tool-selection fields; effective `no-tools/v1` and every connection-verification request require disabled exposure, while a verification request for a `local-tools/v1` binding leaves that durable policy unchanged. Historical assistant `tool_calls` and `tool` result messages remain exact semantic replay and do not expose a current tool registry. Any other tool policy or policy-and-exposure combination MUST fail before transport. The first profile MUST NOT inject an undeclared Provider-specific option. Replay MUST encode system and visible text messages with their exact roles and bytes. A prior assistant round MUST be encoded as one assistant message preserving its exact visible `content`, visible `refusal`, and correlated `tool_calls` as independent fields, including when content or refusal accompanied tool calls; each admitted tool result MUST follow as a `tool` message carrying the exact `tool_call_id`. The connector MUST NOT send provider conversation state or hidden reasoning content as continuation authority.

The SSE decoder MUST accept only one correlated choice with index zero. It MUST preserve exact `delta.content` bytes; MUST expose exact `delta.refusal` bytes as a visible refusal observation; MAY expose `delta.reasoning_content` as a reasoning observation without adding it to exact replay; and MUST correlate indexed `delta.tool_calls` fragments while preserving each call ID, function name, and exact accumulated JSON argument bytes. Content, refusal, and tool-call deltas are independent optional fields and MAY coexist in one assistant round. A refusal completed with `stop` MUST be displayed and committed as a completed visible assistant response rather than classified as a transport or protocol failure, and its exact visible bytes MUST remain in replay. Tool-call indexes MUST first appear contiguously in ascending order; later fragments for admitted indexes MAY interleave. An initial role-only delta MAY be ignored. Multiple choices, a changed response ID, a duplicate or missing tool-call ID, inconsistent function names, a non-contiguous first index or fragment for an unintroduced index, or a malformed delta MUST be a typed protocol failure.

Exactly one semantic choice finish reason MUST precede termination. `stop` denotes a completed final-assistant round and MUST fail as contradictory when tool calls were accumulated. `tool_calls` denotes a completed tool-call round and MUST fail as contradictory when no tool call was accumulated; any accompanying content or refusal remains part of the same assistant message and exact replay. `length` denotes a typed incomplete failure: the backend MUST fail the Turn, MUST NOT commit the partial assistant message or tool calls to replay, and MUST NOT publish a Continuation Anchor covering them. `content_filter` denotes a typed failed response with the same replay and Anchor exclusion. An unknown, duplicate, or contradictory semantic finish reason MUST fail closed; the strictly content-free repetition on the final accounting chunk specified below is metadata rather than another semantic finish.

An absent or null `usage` field on an ordinary choice chunk is transport metadata and MUST be ignored. Exactly one non-null final usage record MUST follow the finished semantic choice in a separate accounting chunk. Its `choices` MUST either be an empty array or contain exactly one accounting choice with index zero. An accounting choice MUST repeat the exact already accepted `finish_reason` and MUST carry an object `delta` containing only optional `role` and `content` fields: `role` MUST be absent, null, or `assistant`, and `content` MUST be absent, null, or the empty string. No refusal, reasoning, tool-call, or unknown delta field is permitted on the accounting choice. An accounting chunk MUST emit no text, refusal, reasoning, message-completion, or function-call observation and MUST NOT finish an unfinished round, change its status, or repeat tool execution. `native_finish_reason` remains non-authoritative transport metadata.

The final usage record MUST report non-negative `prompt_tokens`, `completion_tokens`, and `total_tokens`; a non-negative `completion_tokens_details.reasoning_tokens` MAY also be reported. A non-null usage record before semantic finish, duplicate non-null usage in either shape, any ordinary choice after semantic finish, changed response or choice correlation, missing or contradictory accounting finish reason, non-empty or malformed accounting content, any prohibited accounting delta field, negative values, or totals inconsistent with prompt and completion tokens MUST be a typed protocol failure. All existing SSE-event and cumulative-response limits still apply. Both accounting shapes require the same subsequent mandatory `[DONE]` and stream-end sequence; neither establishes a successful or resumable response by itself.

The exact un-named `data: [DONE]` sentinel MUST be the final data payload after both the finished choice and usage record. It is the Chat Completions stream terminal and MUST NOT be treated as JSON. It MUST NOT be missing, repeated, or carry an SSE event name. SSE comments and events with no data field MAY be ignored as transport framing before `[DONE]`; after `[DONE]`, only non-data framing and stream end are permitted. Any other data after `[DONE]`, invalid UTF-8, or stream end without the complete finish, usage, and sentinel sequence MUST be a typed protocol failure rather than a completed Turn.

The connector transport policy MUST set finite connect, response-header, successful-stream-inactivity, error-body-inactivity, and internal event-delivery deadlines. The first runtime policy MUST use 30 seconds for connect, 5 minutes for response headers, 5 minutes for successful-stream inactivity, 30 seconds for error-body inactivity, and 5 minutes for internal event delivery or backpressure. The phase clocks MUST operate as follows. A connect clock starts when each new connection establishment begins and ends when that HTTPS connection becomes usable; a reused connection has no new connect phase. A response-header clock starts when each HTTP attempt is dispatched, includes any connection establishment, ends when complete response headers are accepted, and never resets. After successful headers, successful-stream inactivity starts before the first body chunk and resets on each non-empty raw HTTP body chunk, including heartbeat, comment, or partial SSE framing bytes, before semantic decoding. It MUST NOT require a completed SSE event to prove transport progress. After non-success headers, error-body inactivity starts before the first error-body chunk and resets on each non-empty raw HTTP body chunk before retention, truncation, or decoding. An empty chunk MUST NOT reset either inactivity clock. Each connector-neutral observation MUST start a fresh absolute event-delivery wait when it is ready for handoff and complete that wait only when the downstream consumer accepts it; network input, prior delivery, or partial downstream progress MUST NOT reset that wait, and the next observation receives a new wait. Event delivery MUST therefore remain separately bounded even while network input is active. Expiry MUST identify the failed phase.

An absolute model-request deadline belongs to the Yo-managed agent policy rather than the effective binding or connector identity. It MUST be optional and MUST default to absent. When supplied, it MUST start once for the logical model request, MUST include bounded connector-internal retry attempts, and MUST NOT reset on received bytes, decoded events, or retry. A separately admitted later model request receives a fresh deadline. Changing runtime deadline policy MUST NOT open a binding epoch. A non-agent connection-verification caller MUST instead provide an explicit finite absolute deadline; the first verification policy MUST use 10 minutes together with the transport deadlines above. Cancellation MUST interrupt every wait regardless of whether an absolute deadline exists.

Transport policy MUST also set maximum error-body bytes, SSE-event bytes, SSE-event count, tool-call count, cumulative response-content bytes, cumulative refusal bytes, cumulative reasoning bytes, and cumulative function-argument bytes. The connector MUST enforce those bounds while reading rather than after buffering an unbounded value. Deadline expiry, oversized HTTP or SSE data, event-count or cumulative response overflow, and cancellation during decoding MUST terminate with a typed failure and MUST follow the same partial-response retry prohibition below.

An HTTP status that explicitly reports throttling or temporary service failure MAY be retried within a bounded policy before any response item is admitted. Connection ambiguity after request transmission and any failure after the first response item MUST NOT be retried automatically. Every retry attempt MUST keep its own request correlation; the connector MUST never repeat a tool result or hide a partial stream behind a replacement response.

## Admitted PNG request projection

An effective binding carrying a separately reviewed image-input profile may
use this connector only after complete-envelope admission by the service owner
and the connector's defensive check. The initial additional service envelope
is `openrouter-free-png-advisory/v1` from `agent.model.service-binding`; the
connector remains provider-neutral and acquires no Provider-named dialect or
implicit capability. An absent profile retains the existing text-only bytes
and rejection of image-bearing inputs. An unknown or mismatched profile fails
before transport, and no remote capability flag supplies executable policy.

For each admitted multimodal user input, the connector emits exactly one
`user` message whose `content` is the original ordered array. Each text part is
`{"type":"text","text":<exact text>}`; each image part is
`{"type":"image_url","image_url":{"url":<PNG data URL>}}`. The data URL is
`data:image/png;base64,` followed by canonical padded standard base64 of the
already admitted immutable PNG snapshot. Repeated images remain separate
occurrences. No path, remote URL, re-read, resize, thumbnail, generated caption,
reordering or deduplication may replace those bytes. Historical multimodal
inputs use the same exact projection on every continuation and fresh-process
resume. System, assistant, refusal, tool-call and result semantics remain as
specified above; hidden reasoning and provider-private replay remain excluded.

Image-bearing summary sources use the same array encoding with their exact
versioned manifest as the first text part followed by the exact ordered image
occurrences. The connector validates the complete source manifest and matching
snapshot descriptors under the managed-loop summary contract before transport.
The existing ordinary-input limits, separate 64-image/16-MiB complete summary
source limits, and request/replay limits all apply. The first excess rejects;
no truncation, split summary, dropped occurrence or retry repairs an oversized
request. The request-local tools-disabled rule also applies to image summaries.

The admitted service policy carries only its closed declared request options;
the connector serializes them without permitting replacement of model,
messages, streaming, usage, output-cap or tool-exposure fields. For the initial
service envelope these are the exact routing fields owned by service-binding,
and they remain present even for requests with no images. Arbitrary optional
parameters remain unsupported. The image-free tokenization projection retains
every serialized non-image field and each image-bearing user message, replacing
its content array with the ordered text parts only; image parts and their data
URLs are omitted rather than counted as text or replaced with fake captions.
The connector separately returns typed immutable descriptors for every image
occurrence in request order. The selected backend policy supplies advisory
image cost without inspecting arbitrary JSON or making a remote estimate.

PNG admission does not relax the existing streaming grammar, bounded transport,
usage, cancellation, or failed/incomplete-response Anchor exclusion. The
additional image profile permits no automatic resend or provider/model fallback,
including before the first response item; this narrows the general bounded
pre-response retry permission for this profile. A JSON error in an HTTP 200
response or a stream error is a failed request, never a completed assistant
response or evidence that images should be removed. The additional projection
MUST activate with its compatible service-binding and managed-loop accounting
revisions before it can admit image transport.

## Rationale

Chat Completions and Responses share transport concerns but have different message, tool-call, usage, and terminal grammars. A separate exact dialect and provider-neutral connector preserve those differences while allowing the same Yo-managed semantic model loop to use either one. Separating transport progress from an optional agent-owned work budget prevents healthy long-running streams from failing merely because a connector chose a universal wall-clock cap, while finite inactivity, delivery, data, and cancellation bounds still detect stalled work. Treating refusal as visible model output preserves the user's conversation rather than misreporting a normal model decision as infrastructure failure. Requiring the finish, usage, and `[DONE]` sequence while excluding `length` and failed responses from replay prevents a truncated stream from becoming durable model history.

OpenRouter documents a final accounting chunk that repeats a content-free choice and the prior finish reason instead of the standard empty `choices` array ([streaming contract](https://openrouter.ai/docs/api_reference/streaming#the-final-usage-chunk-chat-completions)). Recognizing only that final accounting shape preserves provider-neutral decoding and exact replay without interpreting repeated metadata as another assistant response or tool round. No Provider-named dialect, endpoint inference, fallback, image removal, or automatic resend is introduced.
