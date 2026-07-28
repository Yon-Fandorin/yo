---
schema: methexis.knowledge/v1alpha1
id: agent.runtime.session-turn-activity
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-003
    revision: sha256:ab6c58d06b8265e625a748064ce17cbe1d35d1fb1b89c9ee40f08bb783b889d4
relations:
  depends_on:
    - agent.core.frontend-independent-boundary
---
# Session, Turn, and Activity lifecycle

## Statement

A Session MUST own ordered Turns and retained agent context. A Turn begins when
one user request is accepted for execution and ends only as completed,
interrupted, or failed. Model work, streamed responses, tool calls, tool
results, file changes, approval requests and responses, and agent-requested
user input inside that work are Activities of the same Turn rather than new
Turns.

The first product host MUST allow one active Session and at most one active Turn
in that Session, while the core contract identifies every Session and Turn
explicitly and MUST NOT rely on an implicit global current session. A resource
intended to survive a Turn, such as a background process, is Session-owned even
when an Activity records where it was created.

Resume, fork, list, archive, rollback, and multiple concurrently loaded Sessions
are deferred behavior. The initial identity and ownership model MUST permit
those operations to be added without redefining Session or Turn.

## Rationale

The hierarchy gives tool-rich agent work one clear completion boundary while
retaining enough identity for history, branching, and multi-frontend expansion.
