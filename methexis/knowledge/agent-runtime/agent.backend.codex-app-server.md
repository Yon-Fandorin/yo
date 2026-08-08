---
schema: methexis.knowledge/v1alpha1
id: agent.backend.codex-app-server
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-005
    revision: sha256:55e86343fb7ead85e4764c7fa6c86976d6577359c130c6d970605299c4dd7a3b
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

`yo-cli` selects and wires the backend. The private backend module owns its
child process and deterministic cleanup in coordination with the product
process host. The same core contract MUST have a deterministic fake Agent
Backend for contract and failure tests that do not require Codex installation,
credentials, network access, or nondeterministic model output.

WebSocket transport, remote app-server use, and another delegated Agent
Backend are deferred until their own executable evidence exists.

The Codex binding MUST explicitly declare `backend_managed_state` continuation.
Yo owns the durable transcript, semantic events, correlation records, and
versioned Codex Thread locator, while Codex owns the model-visible conversation
state. Resume MUST reconnect through that locator and verify the returned Thread
identity under the binding's versioned identity schema. A completed resumable
Codex Turn MUST emit a payload-free resumable outcome and Continuation Anchor,
but MUST NOT emit a `model_replay_delta` or `replay_delta_sequence`. Provider
Responses or item identifiers MAY remain correlation evidence and MUST NOT be
misrepresented as Yo exact replay.

## Rationale

App-server supplies an existing coding-agent engine, authentication, tools,
approvals, and streamed events, allowing yo to validate its interface without
reimplementing an agent or coupling its domain contract to Codex.
