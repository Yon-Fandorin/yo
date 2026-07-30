---
schema: methexis.knowledge/v1alpha1
id: agent.backend.codex-app-server
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-005
    revision: sha256:7fa50258cd0b1f42dd8ee113b86810ae821bcb23bb197ea2b0b658e516c91473
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

## Rationale

App-server supplies an existing coding-agent engine, authentication, tools,
approvals, and streamed events, allowing yo to validate its interface without
reimplementing an agent or coupling its domain contract to Codex.
