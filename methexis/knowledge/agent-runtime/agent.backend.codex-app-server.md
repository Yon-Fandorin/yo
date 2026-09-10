---
schema: methexis.knowledge/v1alpha1
id: agent.backend.codex-app-server
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-005
    revision: sha256:94cd2c207ef7870d7b336bf2e065f736c6569671ae3ef1644ac5141d0638d06c
relations:
  depends_on:
    - agent.core.frontend-independent-boundary
    - agent.runtime.session-turn-activity
  constrained_by:
    - tui.runtime.process-termination-coordinator
---
# Initial Codex app-server backend

## Statement

The first real Agent Backend MUST adapt a locally installed
`codex app-server` over its default stdio JSONL transport. The adapter MUST
perform initialization and a protocol-version compatibility check, MAY
negotiate additional capabilities, MUST fail explicitly on incompatibility,
map Codex Thread/Turn/Item messages into yo Session/Turn/Activity semantics,
and keep all Codex-specific wire types private to the backend boundary.

`yo-cli` selects and wires the backend. The independent
`yo-backend-delegated-codex` crate MUST depend on `yo-backend` for bounded
child-process JSONL and deferred-message mailbox mechanisms and on the provider-neutral
`AgentBackend` specialization in yo-core. It MUST own Codex-specific launch policy, wire
correlation, semantic translation, and deterministic cleanup in coordination
with the product process host, and MUST NOT make yo-core depend on Codex wire
behavior. The same core
contract MUST have a deterministic fake Agent Backend for contract and failure
tests that do not require Codex installation, credentials, network access, or
nondeterministic model output.

WebSocket transport and remote app-server use are deferred until their own
executable evidence exists.

The Codex binding MUST explicitly declare `backend_managed_state` continuation.
Yo owns the durable transcript, semantic events, correlation records, and
versioned Codex Thread locator, while Codex owns the model-visible conversation
state. Resume MUST reconnect through that locator and verify the returned Thread
identity under the binding's versioned identity schema. A completed resumable
Codex Turn MUST emit a payload-free resumable outcome and Continuation Anchor,
but MUST NOT emit a `model_replay_delta` or `replay_delta_sequence`. Provider
Responses or item identifiers MAY remain correlation evidence and MUST NOT be
misrepresented as Yo exact replay.

## Immutable image input over local stdio

The Codex adapter MAY admit the existing immutable PNG input contract through
`codex.app-server/inline-png-input/v1alpha1`. This extension changes neither the neutral
snapshot nor structured-input persistence. It does not add managed token counters
or Yo replay to Codex's backend-managed continuation.

Support MUST require both an explicitly reviewed app-server image-wire version
and the exact selected model's explicit image modality. The initial reviewed
executable version is `0.153.4`; ordinary protocol compatibility or a warning-and-
continue version decision MUST NOT imply this image capability. Before publishing
capability after create, resume, or native model rebind, the adapter MUST match the
returned model identity to a unique row in the same initialized client's bounded,
complete `model/list` observation. It MUST retain the model identity with that
observation and MUST NOT use a default row, a display name, another model, or the
previous binding's capability. Unknown versions, an absent row or field, malformed
or conflicting modality evidence, or an incomplete observation yield typed Unknown.
A valid explicit modality list without `image` yields typed Unsupported. Only a
valid explicit `image` entry and reviewed wire support yield Supported. Missing
`inputModalities` MUST NOT inherit upstream schema defaults. Capability uncertainty
MUST NOT silently discard images or turn into a claim of text-only model support.

For `turn/start` and `turn/steer`, the adapter MUST project the existing ordered
model input parts into the request's `input` array. A text part is
`{"type":"text","text":<exact projected text>}`. An image part is
`{"type":"image","url":"data:image/png;base64,<canonical padded RFC4648 PNG bytes>"}`.
No `detail` override is sent. Empty inter-image text spans are omitted, repeated
image occurrences remain separate, and the existing skill trailer stays in its
established final text position. The bytes MUST be those of the admitted immutable
snapshot, without reopening a path, re-normalizing, uploading a file, fetching a
remote URL, or using `localImage`. Literal markers remain ordinary text. The
adapter hands these exact PNG bytes to Codex; it MUST NOT claim that Codex's
subsequent model preparation preserves original pixels or applies no resizing.
Existing text-only projection and all unrelated request fields remain unchanged.

The adapter MUST enforce the common input limits: at most 16 occurrences, at most
33 ordered parts, canonical PNG bytes at most 9,437,184 per input charged per
occurrence, and the existing snapshot/source/dimension/encoded-input admission
limits. Codex stdio JSONL MUST have a separate bounded envelope of 33,554,432
encoded bytes per inbound or outbound message, excluding the line terminator.
The first excess byte MUST fail before publishing an inbound value or sending an
outbound message; counting and arithmetic MUST be checked. The larger Codex-only
receive envelope accommodates echoed user-message media and MUST NOT relax
other backend, decoder, attachment, or untrusted Markdown preview limits.
Outbound admission MUST cover the complete JSON-RPC request, including escaped
text, references, skill trailer, base64, policy, identity fields and framing;
individual image bounds alone are insufficient. No bound is a hard process RSS
or upstream service limit guarantee.

Unknown, unsupported, or excessive image input MUST be rejected before Codex
receives a turn request and preserve the existing draft and immutable attachments
through the established submission identity. Rejection MUST NOT retry with stripped
images, change models, submit a caption, or create an accepted-request record.
After backend acceptance, ordinary failure and uncertain-continuation rules apply;
no provider error authorizes an automatic resend or synthetic successful Anchor.
Ordinary resume and native model transitions MUST revalidate the selected target's
image capability against the complete durable Yo input-image history. When that
history contains images, or its absence cannot be established, Unknown or
Unsupported capability MUST reject executable continuation before native state is
resumed, forked, or rebound, preserving the source binding and read-only history.
A text-only next input MUST NOT bypass this retained-image check. Every subsequent
turn also requires image support while retained Yo input-image history exists;
queued and drafted image input additionally use the newly selected capability.
The adapter MUST NOT permit upstream to replace those retained images with
unsupported-media placeholders. Complete historical input evidence may establish
that a Session is text-only; a model name, an empty replay delta for backend-managed
state, or missing history evidence cannot. This history evidence does not transfer
Yo replay to Codex or assert knowledge of private backend context.

## Rationale

App-server supplies an existing coding-agent engine, authentication, tools,
approvals, and streamed events, allowing yo to validate its interface without
reimplementing an agent or coupling its domain contract to Codex. Keeping the
adapter in its own crate also prevents the semantic core from becoming the
ownership boundary for one host's process and protocol.
