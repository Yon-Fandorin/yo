---
schema: methexis.knowledge/v1alpha1
id: agent.runtime.active-turn-input
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-004
    revision: sha256:758a6808866b5e829a0587c25714535461e007977c7ca5b36292c91197438329
relations:
  depends_on:
    - agent.runtime.command-event-boundary
---
# Input during an active Turn

## Statement

Normal prompt submission while a Turn is active MUST be interpreted and
submitted as a steer request for that identified Turn in the initial TUI. A
response to an outstanding approval or agent-requested input is an Activity
response, not a steer or a new Turn.

Queueing input for a later Turn is a distinct deferred operation. When the
selected backend cannot steer, `yo-core` MUST return an explicit unsupported
result and MUST NOT silently reinterpret the input as queued work.

## Rationale

Explicit steer and queue meanings prevent user input from changing temporal
intent based on hidden backend capability while supporting immediate
correction of active agent work.
