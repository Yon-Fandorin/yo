---
schema: methexis.knowledge/v1alpha1
id: agent.backend.execution-topology
kind: decision
owner: agent-runtime
sources:
  - id: agent.backend-007
    revision: sha256:e8a41776fd9854d6651c837d1a2e24683a1b12606ac0f9bdcf0757631c9e73e0
relations:
  depends_on:
    - agent.core.frontend-independent-boundary
---
# Agent backend execution topology

## Statement

An Agent Backend MUST be classified independently along orchestration
ownership, connector, execution target, transport, workspace-host, and
tool-execution-host axes.
A Delegated Agent Backend connects to a coding-agent host such as Codex
app-server, Grok Build ACP, or Kimi Code; that host owns its agent loop, tool
execution, and backend Session. A Yo-managed Backend keeps those
responsibilities in yo and uses a Model Connector to reach a service such as
OpenAI or Kimi.

`Provider` MUST name a model service rather than a delegated coding-agent
process. `Local` and `Remote` MUST describe execution placement, and stdio,
SSH, WebSocket, HTTP, and SSE MUST describe transport; neither dimension
creates another semantic backend kind. Every Agent Backend MUST report the
exact boundary that it can observe for Request diagnostics, through its
Connector where one exists, and MUST NOT claim visibility into a downstream
request owned by another process or service.

Generic backend lifecycle, capability, failure, evidence, and replay types MUST
live in the independent `yo-backend` foundation crate. Bounded child-process
JSONL, stderr retention, request-ID allocation, and deferred-message mechanisms
MAY be shared there, but host wire interpretation and Yo semantic state MUST
NOT enter that foundation. yo-core MUST specialize `BackendAdapter` as its
provider-neutral `AgentBackend` port and MUST NOT depend on a concrete backend.

Concrete backends MUST remain flat independent crates: `yo-backend-managed`,
`yo-backend-delegated-codex`, and `yo-backend-delegated-grok`. Each depends on
the foundation and yo-core specialization. The process host selects and
constructs an admitted adapter. The current local delegated adapters are Codex
app-server and Grok Build ACP.

## Rationale

Keeping ownership, vendor, placement, and wire protocol orthogonal avoids
local-only backend types and lets the same Session semantics cover local Codex
and Grok processes, a remote agent host, and a yo-owned model loop. Independent
adapter crates keep host protocol churn out of the semantic core and allow a
new host without adding another concrete backend dependency to yo-core.
