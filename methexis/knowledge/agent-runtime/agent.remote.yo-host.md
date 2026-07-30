---
schema: methexis.knowledge/v1alpha1
id: agent.remote.yo-host
kind: decision
owner: agent-runtime
sources:
  - id: agent.remote-001
    revision: sha256:053c6ea0be4cdc7c27ba6fdcb331fbc061682785e15b540653ecd0508a8548c7
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.observability.session-journal
    - agent.observability.view-projections
    - agent.storage.session-repository
  constrained_by:
    - agent.core.frontend-independent-boundary
    - agent.runtime.command-event-boundary
---
# Local and remote Yo Host

## Statement

Remote-machine use MUST remain an architectural constraint. Local execution
MUST be the in-process placement of the same Yo Host contract used remotely,
not a separate local-only design. A Yo Host MUST own the Session Engine, the
Host component that coordinates Session command processing and lifecycle,
together with the workspace, Agent Backend, tools, Session Journal, and
Session Repository. A Frontend MUST send
intents and consume projections without owning Host paths, processes, PTYs, or
operating-system signal policy.

The Yo Session Protocol MUST be versioned, ordered, capability-aware, and
independent of its network transport. A remote Session MUST continue when its
client disconnects, and reconnect MUST request the Journal suffix after the
client's last accepted sequence. When that range crosses a durable gap, the
Host MUST carry the explicit gap and the first complete recovery snapshot
instead of claiming a continuous suffix. Without that snapshot, the Host MUST
report that durable history remains unavailable. WebSocket MUST be the first
remote transport so terminal and future browser or Tauri frontends can share
the protocol. Adding gRPC later MUST NOT redefine Session meaning. External
binding MUST require explicit activation plus authenticated encrypted
transport; an unauthenticated external bind MUST NOT be a default. A remote
Yo Host MUST remain distinct from a local Host using a Connector to reach a
remote delegated-agent target.

## Rationale

Treating local mode as embedded Host placement keeps remote workspaces,
long-running sessions, reconnect, and future GUI clients open without
virtualizing remote files and processes inside every frontend.
This contract constrains a future remote capability; implementation still
requires a real remote consumer under
`agent.runtime.command-event-boundary`.
