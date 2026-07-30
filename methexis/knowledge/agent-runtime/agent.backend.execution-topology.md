---
schema: methexis.knowledge/v1alpha1
id: agent.backend.execution-topology
kind: decision
owner: agent-runtime
sources:
  - id: agent.backend-007
    revision: sha256:368e802dc3b559f2d45737ee31509f0b8deecf5afe6954cd17b2fb12af7409d2
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
app-server or Kimi Code; that host owns its agent loop, tool execution, and
backend Session. A Yo-managed Backend keeps those responsibilities in yo and
uses a Model Connector to reach a service such as OpenAI or Kimi.

`Provider` MUST name a model service rather than a delegated coding-agent
process. `Local` and `Remote` MUST describe execution placement, and stdio,
SSH, WebSocket, HTTP, and SSE MUST describe transport; neither dimension
creates another semantic backend kind. Every Agent Backend MUST report the
exact boundary that it can observe for Request diagnostics, through its
Connector where one exists, and MUST NOT claim visibility into a downstream
request owned by another process or service.

## Rationale

Keeping ownership, vendor, placement, and wire protocol orthogonal avoids
local-only backend types and lets the same Session semantics cover a local
Codex process, a remote agent host, and a future yo-owned model loop.
