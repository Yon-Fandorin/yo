---
schema: methexis.knowledge/v1alpha1
id: agent.backend.yo-managed-model-loop
kind: decision
owner: agent-runtime
sources:
  - id: agent.backend-008
    revision: sha256:521fb41f6f30738aa5e8289437f4d3b4f48344b88e162a89e2088a2994616dfd
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.connector.kimi-chat-completions
    - agent.connector.openai-chat-completions
    - agent.connector.openai-responses
    - agent.observability.session-journal
    - agent.runtime.command-event-boundary
    - agent.runtime.session-turn-activity
    - agent.session.continuation-lineage
    - agent.tool.local-execution-boundary
---
# Yo-managed model and tool loop

## Statement

The Yo-managed Agent Backend MUST live in the independent `yo-backend-managed` crate, implement the existing `AgentBackend` semantic port, and own the model loop, tool execution coordination, and model-visible context. `yo-core` MUST own the shared semantic, model-service, Connector, and tool types and ports without owning or depending on the concrete loop. The effective binding MUST select one admitted Model Connector and exact API dialect before a Turn starts. A Model Connector MUST own remote request and stream protocol and MAY additionally own only its explicitly contracted provider-private replay codec and validation; it MUST translate both dialect events and any validated private visible projection into connector-neutral observations consumed by the loop. Neither `yo-cli`, a frontend, nor the connector may become the agent-loop owner. The same loop admits the separately contracted OpenAI Responses, provider-neutral OpenAI Chat Completions, and Kimi Chat Completions connectors through their dialect-derived identities. It MUST NOT probe another dialect, fall back to another connector, or branch on Provider identity.

For each accepted Turn, the backend MUST project the committed semantic Session history plus the new user input into the selected API dialect. Text deltas MUST become `ModelWork` Activities through the existing message segmentation and terminal-seal path. A model function call MUST preserve its wire call identity, function name, and exact accumulated argument bytes. It MUST become a correlated Tool Activity even when validation rejects it; invalid JSON, schema mismatch, unknown or duplicate identity, unavailable tool, and argument-bound failure MUST terminate that Activity with the typed validation failure and no effect. Validation MUST succeed before approval, admission, or dispatch. Approval and execution MUST use the frozen registry, admission policy, and execution-host boundary; the model service MUST NOT directly execute local workspace tools.

The backend MUST record each function call and its exact tool outcome before submitting a corresponding function-call output through the selected dialect in the next model request. Multiple calls returned by one response MAY execute concurrently only when the tool scheduler proves their approval and mutable resource leases independent; otherwise they MUST execute in model order. Results MUST be returned in stable call order regardless of execution completion order. A missing, duplicate, or mis-correlated call or result MUST fail the Turn.

The loop continues across model response, local tool execution, and tool-result submission until the model emits a final assistant message, cancellation is accepted, a bounded model-round limit is reached, or a typed failure occurs. One active Turn remains the Session limit. Cancellation MUST stop outstanding connector work promptly, prevent new tool execution, seal active Activities as interrupted, and run explicit connector and tool cleanup.

The loop owns any absolute model-request work deadline. That deadline MUST be optional and MUST default to absent. When the agent supplies it, the deadline MUST begin once for one logical model request, MUST cover every bounded connector-internal retry for that request, and MUST NOT reset on transport bytes, model output, decoded events, or retry. The next model request after a tool result, or a separately admitted request after an earlier failure, MUST receive a fresh deadline. A whole-Turn wall-clock budget is a separate optional cancellation policy and MUST NOT be inferred from the per-request deadline. Absence of either absolute budget MUST NOT disable the connector's finite transport-progress, event-delivery, data, round-count, cancellation, or cleanup bounds. Runtime deadline policy MUST remain outside the effective binding and MUST NOT open a binding epoch.

The backend MUST also supply one provider-neutral typed cache-affinity hint with every ordinary Session model request. Its exact value is the canonical lowercase hyphenated UUIDv7 string of that Yo `SessionId`; it is therefore stable across exit and exact resume, distinct across Sessions, 36 ASCII bytes, and independent of Provider, Account, Model, credential, prompt contents, and response state. Connection verification has no Session and is owned separately by the model-service verification coordinator. The hint is request context only: it MUST NOT enter complete binding equality, a binding epoch, replay identity, Session semantics, user-visible output, Transcript, Request trace, logs, errors, or diagnostics. The backend passes the typed value without branching on Provider; each selected Connector owns whether and where its reviewed dialect serializes the value. A Connector MUST NOT forward it to an endpoint whose contract does not admit it.

Provider response IDs, cache handles, and conversation IDs MAY be retained as diagnostic correlation but MUST NOT be the only continuation locator. A Yo-managed binding MUST explicitly declare `exact_replay` with `local_client` as the current executor rather than provider-native resume, and its complete effective profile MUST carry the exact `replay_profile`. Executable continuation without a checkpoint MUST reconstruct the replay boundary named by the newest durable Continuation Anchor from the Session Journal. When a valid checkpoint exists, it is the model-context reconstruction root: its synthetic user-role portable body and exact retained groups replace only the summarized prefix, and later replay deltas are applied through the newest Anchor in the checkpoint's successor context epoch. The checkpoint's source Anchor remains provenance and MUST NOT be mistaken for the current reconstruction boundary. Endpoint, API dialect, Provider, Account, Model, connector identity, or replay-profile changes still open a new binding epoch. A committed mid-Turn function call, tool result, partial stream, private assistant fragment, or other suffix beyond the selected current-epoch Anchor MUST remain diagnostic and MUST NOT become automatic continuation input. With neither a durable Anchor nor a valid checkpoint, the Session MUST follow the continuation contract's read-only fallback rather than constructing replay input. Exact replay MUST preserve message roles and order, exact visible text, function-call and tool-result relationships, and the recorded system/tool contract. `semantic-only/v1` forbids provider-private items. `kimi-private-local-plaintext/v1` declares schema `kimi.assistant-message/v1alpha1` and MAY preserve its item only when the Connector has decoded and losslessly validated the private bytes, returned their connector-neutral validated visible projection with the opaque envelope, and satisfied visibility exclusion, byte bounds, binding scope, durable encoding, and exact request projection. Such an item is Session replay authority only for the same exact binding identity and replay profile; it never becomes generic visible history, provider-native state, or a frontend observation. A checkpoint may deliberately drop a private item only from its summarized prefix with the contracted disclosure. A retained group that originally required a private item remains indivisible and MUST retain that exact Journal-backed item; the synthetic user-role checkpoint body itself requires no provider-private assistant counterpart. Provider cache state and every uncontracted private field remain excluded. After a checkpoint, the 4096-item and 64-MiB cumulative replay bounds apply to the synthetic body, retained groups, and successor-epoch deltas, never again to the replaced pre-checkpoint prefix.

No Continuation Anchor may cover a partial model stream, an uncommitted tool result, an uncertain request, a failed final response, or a Kimi private-replay round whose required private assistant item is missing or not durably committed with its semantic replay delta. Usage and the exact effective binding MUST be attributed to the model response that produced them, including when the model changes inside one Yo Session.

For every completed provider response and model round, the backend MUST persist exactly one terminal `ModelWork` Activity usage receipt. Its content MUST use exact schema `yo.model-usage-receipt/v1`, retain the response ID, positive round, Provider, Account, Model, Connector, API dialect, and normalized endpoint attribution already emitted by the current Activity, and retain the Connector's input, output, reasoning, and total token counts without redefining their meaning. The receipt's `cache_read_input_tokens` MUST be exactly one closed availability object: `reported` carries non-negative `tokens` plus the Connector-defined non-empty versioned `source_profile`; `absent` carries that source profile without tokens; and `unsupported` carries neither tokens nor a source profile. Reported zero MUST remain distinct from absent, and absent MUST remain distinct from unsupported. A Connector without an accepted cache-read meaning MUST produce unsupported rather than guessing from a prompt prefix, cache-affinity key, Provider identity, or repeated content. The receipt is response telemetry in the durable Activity and MUST NOT become request context, binding identity, replay authority, or a second response record. It MUST exclude raw request or response bodies, credentials, the cache-affinity value, private reasoning, inferred cache writes, inferred uncached tokens, cost, and a separate inferred hit flag. Older Activities without this receipt schema remain readable under their existing generic Activity content semantics; no Journal envelope or record variant changes.

The selected model catalog entry MUST provide an input-token limit and the exact tokenizer profile used by an injected token counter. Its optional positive `max_output_tokens` records a known hard maximum; absence records that Yo does not know a trustworthy numeric maximum and is valid only when the selected Connector contract permits omission of its output-limit field. Presence or absence remains complete-binding and epoch identity. A binding with a known maximum may select a smaller positive request-local output cap without changing that identity; a binding without one MUST omit the output-limit field and MUST NOT invent a numeric cap. After provider-private assistant replacement and request-local tool projection, the backend MUST pass the exact complete Connector-produced tokenization payload to the counter before every model dispatch, including a dispatch after tool results. With a known maximum, it MUST serialize the final selected cap, recount that final exact payload, and dispatch only when the count plus the cap fits `input_token_limit`; the cap MUST NOT exceed the known maximum. Without a known maximum, it MUST count the exact payload with the output-limit field absent and dispatch only when that input count is strictly below `input_token_limit`. There is no provider-neutral minimum-output budget. Before any dispatch, the backend MUST also prove that the retained replay prefix plus the pending Turn delta remains within the cumulative replay item and byte bounds. Provider-side implicit caching MAY reduce billing but MUST NOT change exact replay or context admission.

An ordinary `local_client` exact-replay Session MUST persist one closed `yo.context-policy/v1alpha1` independently of complete binding identity. Its durable record kind is `context_policy_changed`; it carries a positive Session-global policy revision, enabled state, exact strategy, warning percent, trigger percent, optional retained-raw percent, and optional retained-raw maximum tokens under the persistence contract's closed domains. Revision 1 MUST precede the first model request, and every replacement increments by one. The default MUST enable `portable-summary/v1alpha1`, warn at 85 percent, trigger compaction at 90 percent, and bound only additionally selected older raw semantic groups to the smaller of 10 percent of `input_token_limit` or 65,536 tokens. `exact-replay-only/v1alpha1` MUST reject rather than compact at the trigger. Warning MUST remain an observable pressure state distinct from both admission and compaction. The percentages apply to the exact complete Connector-produced input before every model request, including after tool results, approval outcomes, or steering input; Provider cache-read accounting MUST NOT subtract tokens from context occupancy. Invalid bounds or an unknown strategy MUST fail closed. A policy replacement MUST append the durable record and become effective before the next model request. It MUST NOT change complete binding identity or open a binding epoch. A replacement whose trigger is already reached MUST run the selected admission before that next dispatch.

Automatic pressure handling and explicit idle `/compact` with optional user guidance MUST invoke the same compaction pipeline and failure rules. Disabled or exact-only policy MUST reject manual compaction. During an active Turn, the pipeline may cut only after a completely committed correlated semantic group, including a tool result or approval outcome, and before the next ordinary Turn model request. It MUST NOT cut a model response, pending tool effect, incomplete group, or accepted Turn request. The portable strategy permits exactly one semantic summary request through the same selected binding and Model with tools disabled. It MUST select only an older committed visible semantic prefix and MUST retain the current user input, every unprocessed steer or approval, the newest complete semantic group, and the canonical system and tool contract. Those mandatory protected items are exempt from the optional retained-raw budget and MUST NOT be truncated to satisfy it. Additional newest groups MAY be retained only while their exact incremental Connector input stays within that budget. A function call and result, an approval request and response, and every other correlated semantic group are indivisible; the pipeline MUST NOT retain an entire active Turn merely because it is active. Provider-private bytes, private reasoning, credentials, and uncommitted effects MUST NOT enter the summary request or any artifact payload. Their schema, presence, byte count, and loss class MAY enter the deterministic disclosure.

The model-generated portable body MUST contain exactly one document heading `Context Checkpoint` followed in order by sections `Current Objective`, `Active Constraints`, `Decisions`, `Verified Progress`, `Current State`, `Unknown or Unverified`, `Next Actions`, and `Critical References`. Section prose MUST follow the conversation language. Each section MUST contain either supported non-empty prose or the exact single paragraph `None.` when the source contains no supported fact for that section; omission, invented filler, and an empty section are invalid. The model MUST NOT regenerate `context_epoch`, source coordinates, token measurements, retained-group identities, artifact hashes, loss disclosure, modified-path state, or any other value already owned by the runtime or Journal. Missing, duplicate, reordered, or additional structural headings make the response malformed. Live structured TODO or plan state MUST be reattached from its owner rather than copied into the body; the body MAY preserve only reasoning that constrains its order or next action.

When an eligible old large visible tool output belongs to the summarized prefix, the checkpoint MAY record a `yo.context-artifact-receipt/v1alpha1` as non-model-visible disclosure. The receipt MUST bind an exact SHA-256 content hash, positive byte count, bounded ASCII media kind, source context epoch, and exact Journal source sequence. The output leaves successor model input only because its whole prefix is summarized; neither the receipt nor a placeholder replaces it in the synthetic body, retained replay, or Connector payload. Raw bytes MUST remain in the Session Journal and MUST NOT be copied into the summary or checkpoint. The first contract exposes the receipt only to operators under both `local-tools/v1` and `no-tools/v1`; it adds no replay-item variant, model-visible artifact-read tool, frozen-registry change, expiry, path, or cross-Session authority. A later bounded retrieval operation requires a separate tool-registry contract and implementation Slice.

After a valid bounded summary, the backend MUST build and exactly recount the complete successor Connector payload and apply the same trigger once. Pressure admission has exactly three decisions: `Admit`, `Compact`, and `Reject`; warning is a separate observable boolean state and never a fourth decision. This final admission is the hard bound even when the mandatory protected set alone exceeds the optional retained-raw budget; only `Admit` permits the sole semantic writer to commit. A second `Compact` MUST become typed `Reject`, and `Reject` MUST remain rejection. The backend MUST preserve the mandatory set intact rather than truncate it or claim compaction success. The atomic `yo.context-checkpoint/v1alpha1` commit contains the positive successor `context_epoch`, previous context epoch, binding epoch, source Anchor and exact semantic boundary, the exact canonical system and tool replay contract, portable body, exact inline retained replay groups and their source identities, artifact receipts, loss disclosure, exact before/after input counts, summary request usage, and policy revision. Context epoch is Session-global: the initial binding starts at 1, a later binding inherits the current value unchanged, and only this same-binding checkpoint increments it by exactly one. In the reconstructed Connector input, the portable body is exactly one synthetic `user` message followed by the checkpoint's exact inline retained replay groups in their original order under that checkpointed replay contract; it is not an assistant message and therefore requires no provider-private assistant item. The backend MUST swap its model-visible replay and dispatch the next model request only after that durable commit returns. When an active Turn crosses one or more checkpoints, each later request uses the newest successor context epoch. At `TurnFinished(completed)`, its sole `model_replay_delta` MUST use that newest epoch and contain exactly the model-visible suffix committed after the newest checkpoint record, including the final assistant group; it MUST NOT duplicate the checkpoint's replay contract, synthetic body, retained groups, or any earlier Turn item. The delta, resumable outcome, and Anchor MUST reference the latest accepted request in that successor epoch, while earlier accepted requests in the Turn remain historical evidence in their original epochs. A committed checkpoint is sufficient to reconstruct model context before another request. If recovery finds a later accepted request without its completed current-epoch Anchor, it MUST preserve the usual uncertain-request read-only boundary rather than treating the checkpoint as proof that the request is safe to repeat. Compaction changes no Provider, Account, Model, Connector, endpoint, replay profile, complete binding identity, or binding epoch. It also MUST NOT alter binding-transition cache evidence or infer Provider cache preservation or loss; only cache-read tokens reported by a later actual ModelWork usage receipt are evidence of a cache read. Summary failure, malformed output, cancellation before commit, another `Compact`, typed `Reject`, artifact-integrity failure, or checkpoint durability failure MUST leave the prior Journal and context epoch authoritative and MUST cause typed context rejection without another semantic summary request, retry, fallback, Provider or Model change, steer, partial checkpoint, or hidden lossy continuation. The existing request-local output admission, request deadline, transport-progress bounds, cancellation, and cleanup rules apply; compaction introduces no separate deadline or Provider branch.

Local input or replay-capacity exhaustion before a final assistant answer, including exhaustion after a tool result or approval decline, MUST fail the current Turn with typed capacity evidence, finish it as non-resumable, and reject a later Turn on that binding. It MUST NOT complete a final-less Turn successfully or lose the terminal Turn record. A Connector-reported incomplete response retains its separately contracted request-failure semantics and does not by itself latch local context admission. If a final assistant answer and every required semantic and provider-private item have already passed their individual validation and bounds, but applying that complete final replay delta to the retained prefix exceeds cumulative replay capacity, the backend MAY preserve the visible Turn as completed and non-resumable without a Continuation Anchor. A missing, malformed, mismatched, or individually unbounded provider-private item remains a pre-acceptance failure and MUST NOT use that exception. Neither path may silently discard, truncate, redact, or summarize required semantic or private state. The preceding context-checkpoint contract is the only lossy compaction path within one binding and advances only its context epoch. Any other lossy or private-state-dropping handoff remains an independently reviewed, user-visible binding transition.

Tool arguments and outputs MUST pass the local tool boundary's semantic-admission gate before they become Activities, later model input, or a replay delta. A provider-private assistant item MUST come only from the selected Connector's successfully completed, correlated response. The Connector alone MUST decode and validate the provider-private schema and return the bounded opaque envelope together with a connector-neutral validated visible projection. Without decoding Provider fields, the backend MUST validate the envelope's declared schema identity, binding epoch, and bounds and MUST compare the Connector-supplied projection exactly with the semantic replay group; any mismatch fails before acceptance. The backend MUST persist visible and private replay together as one semantic replay record and MUST NOT attach either payload to the payload-free resumable-outcome correlation record. Private bytes remain in the user-only local Session Repository, are not encrypted by the first implementation, and MUST be excluded from Transcript, Request trace, debug formatting, logs, errors, and diagnostics.

A future `managed_server` executor MAY load the same validated replay prefix and assemble the next model request on a Yo-managed Session service. It does not define a second replay meaning and MUST use the same replay contract, ordering, bounds, and Anchor boundary as `local_client`. It remains deferred until its remote repository, identity, digest, availability, and retention evidence has an independently reviewed implementation. The current backend MUST NOT advertise it.

## Image-aware request accounting and compaction

For an effective binding with a reviewed `image_input_profile`, its separately
versioned image accounting profile extends the preceding strict text-only counting
rules explicitly. Its closed result carries `quality`,
`policy`, `input_estimate` and `reserve_tokens`; planning count is the checked
sum of the latter two unsigned 64-bit counts. Quality is exactly `exact`,
`verified_upper_bound` or `advisory_estimate`, and the policy is the complete
binding's selected admitted versioned identity. The backend MUST preserve all
four fields through live input, tool-result, approval-outcome, steering, summary,
resume and model-replacement admission, pressure display and checkpoint evidence.
Existing bindings without `image_input_profile` keep their exact previous
counter, bytes, behavior and wire shapes. Exact and verified-bound profiles
do not inherit advisory acceptance. Unknown image capability rejects before dispatch; uncertain accounting quality
does not itself mean unsupported media.

Under `kimi-code-image-advisory/v1`, let T be the selected connector's complete
image-free tokenization projection count after private-assistant and request-local
tool replacement, and N be the total immutable image occurrences in that request.
Input estimate is T + 2000*N. Reserve is 1024 when N is positive and zero
otherwise, charged once for the complete request. These are advisory planning
values, never measured usage or a proved upper bound. The connector supplies a
typed image-free projection and separate immutable image descriptors; the backend
MUST NOT search arbitrary JSON for image-like URLs, count base64 as image cost,
read a path, fetch an image URL or make a hidden remote estimator request.

Warning/trigger percentages and output-budget checks consume the complete
planning count. A known output maximum retains the existing final selected-cap
serialization and recount, admitting only when planning count plus that cap
fits the input limit. An unknown maximum retains omission and strictly-below-limit
admission, now under the explicit quality of the selected policy. Additional raw
retention consumes the difference between complete request planning counts, not
a sum of group counts with repeated reserves. Mandatory protected groups remain
exempt from that optional raw-retention budget. The final successor is recounted
under this same quality and policy; its `Admit` authorizes commit but an advisory
result is not a guarantee of server acceptance. New pressure/checkpoint shapes
record estimate and reserve separately and must label advisory evidence visibly.

Actual provider usage remains immutable telemetry for the exact completed
request; it does not relabel past planning counts or infer per-image cost from
mixed usage. A provider 4xx keeps the existing explicit failed-request and
uncertain-Anchor behavior. HTTP 400 alone is not typed context-overflow proof.
This profile introduces no automatic resend, image stripping, model replacement,
second summary request or fabricated successful compaction. The committed input
and snapshots remain intact, and existing explicit recall/edit/new-session paths
retain attachments. A preparation/admission failure before dispatch preserves
the draft according to the input owner.

When a selected summary prefix contains images, the one tools-disabled summary
request uses the same selected binding/model and one user-role source message
with its distinct typed summary-source content. This request source is not an
ordinary persisted `multimodal_user` input: it admits one manifest plus up to 64
images under the separate summary limits, while each referenced original input
retains its own at-most-33-part domain. Its first text part is the closed versioned source manifest
`yo.image-summary-source/v1`; the following parts are exactly the manifest's
image occurrences, with contiguous unique zero-based image indices in source
order. Manifest message content preserves the original ordered text/image parts,
with each image represented by an index plus canonical hash, byte count, width
and height. The corresponding following typed image part contains the exact
admitted PNG snapshot. No missing or surplus image, metadata mismatch, path,
URL, base64-as-text, thumbnail or generated caption may replace it. Repeated
identical bytes at distinct positions stay distinct occurrences. Existing
text-only summary source encoding is unchanged. The source manifest preserves
visible roles, refusal, function-call IDs/names/arguments and correlated result
relationships without making historical assistant/tool entries top-level
protocol messages. Provider-private bytes remain excluded. Images and embedded
text are untrusted source data, never system authority.

An image-bearing summary source admits at most 64 image occurrences and 16 MiB
of complete canonical multipart source encoding, including manifest, text and
actual image/base64 framing. The selected connector's admitted count/byte limits
also apply. These limits are separate from the 16-image ordinary input cap and
64-MiB replay-prefix cap. A valid replay may not fit summary capacity; the first
excess causes typed rejection with the prior context intact, not truncation,
dropped occurrences, split semantic groups or additional summary requests.

Each summarized image is passed intact to that request and yields exactly one
runtime-derived `image_input_summarized` loss entry under the persistence
contract, including its exact current-Session source location. Retained images
remain inline and exact; current input, pending steer/approval, newest complete
group and the system/tool contract remain mandatory. The successor portable
body is still exactly one plain user-role message followed by exact retained
groups; it contains no embedded image, receipt or replay placeholder. Success
uses the new typed-accounting checkpoint profile; all existing complete-group,
one-summary, final-admission, durable-before-dispatch, cancellation and failure
rules remain in force. Output artifact receipts and retrieval authority do not
change under this input-image extension.


### Closed image summary and pressure projections

The first summary text part contains the canonical object with writer fields
`schema: yo.image-summary-source/v1` and `history`. History flattens the selected
complete groups in order, excluding private envelopes as in the legacy source.
Legacy message entries retain type, role, content and nullable refusal; function
call entries retain type, call_id, name and arguments; output entries retain type,
call_id and output. Their exact strings and valid role/refusal relationships are
unchanged. A multimodal user entry has type `multimodal_user` and ordered `parts`.
A text part has type `text` and nonempty text. An image manifest part has type
`image_ref`, index, sha256, byte_length, width and height in that writer order;
its fields must equal the corresponding following typed PNG snapshot. Indices are
unsigned32-bit contiguous zero-based values in flattened occurrence order, never
hash-based deduplication. Unknown/duplicate fields and null outside the legacy
nullable refusal are rejected. No source paths or private fields are added.

The canonical source-byte metric is the complete UTF-8 JSON object with fields
`role: user` and `parts`; the first part is `{type: text, text: manifest-json}` and
each remaining part is `{type: image, snapshot: canonical-snapshot}`. Compact JSON
uses no extra whitespace, decimal integer scalars, the prescribed field order
and the existing JSON string escaping. Thus the escaped manifest text, every
snapshot/base64 byte and all framing count toward16MiB. This metric is separate
from the final connector request's full framing and admission. With no images,
the existing single-string summary-source representation and bytes stay unchanged.

An image-accounting binding uses exact pressure profile
`yo.context-pressure/v2alpha1`: writer fields are schema, accounting,
input_token_limit, warning_percent, trigger_percent and decision. Accounting has
the same closed quality/policy/input_estimate/reserve_tokens object as the new
checkpoint. Input limit remains positive u64; warning is1..99, trigger2..100 and
warning is less than trigger; decision is admit, compact or reject. The old
input_tokens field is forbidden in this profile. Bind profile selection to the
owning accounting policy even when there are zero images; retain legacy pressure
bytes for bindings without that extension. UI may display planning totals only
with explicit estimate/quality and reserve rather than measured-usage wording.

## Rationale

Owning the loop in `yo-backend-managed` provides a distinct managed backend while keeping `yo-core` as the frontend-independent semantic contract owner. An explicit connector boundary lets multiple API dialects, including a Provider-specific dialect when its wire behavior materially differs, share the semantic loop without adding Provider branches or weakening their distinct grammars. Agent ownership of optional work budgets permits intentionally long model work without weakening transport-stall detection or binding identity. Exact semantic replay avoids coupling durable continuation to a provider's temporary response retention and keeps tool side effects correlated with Yo's own authority. Treating admitted Kimi private reasoning as a separately typed, non-observable replay attachment preserves that boundary while adapting Yo to the current model instead of disabling it merely because its continuation grammar is richer.
