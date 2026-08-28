---
schema: methexis.knowledge/v1alpha1
id: agent.runtime.active-turn-input
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-004
    revision: sha256:01c3a096cd84a7fe7cb0ec260ce6656ade33ad4fd906f2dcadae9aa9cc773ed7
relations:
  depends_on:
    - agent.runtime.command-event-boundary
---
# Input during an active Turn

## Statement

When the initial TUI has observed `TurnStarted(turn)` and has not yet observed
that Turn finishing, a normal prompt submission MUST be interpreted as an
exact-turn steer intent carrying that `TurnRef`. It MUST NOT use a generic
submission whose meaning can be reclassified from newer worker state. The
agent Session MUST admit the intent only if that exact `TurnRef` remains its
active Turn. If the worker has already applied `TurnFinished`, no Turn is
active, or a different Turn is active, it MUST reject the exact-turn intent
explicitly and MUST NOT reinterpret it as `StartTurn`--even when the frontend
has not yet polled the finishing event.

Dispatch backpressure and retry MUST retain the same exact-turn intent and
`TurnRef`; they MUST NOT turn it into later work. A generic new-Turn submission
path MUST be used only for input that the frontend did not classify against an
observed active Turn. A response to an outstanding approval or agent-requested
input remains an exactly correlated Activity response, not a steer or a new
Turn.

Queueing input for a later Turn is a distinct deferred operation. When the
selected backend cannot steer, `yo-core` MUST return an explicit unsupported
result and MUST NOT silently reinterpret the input as queued work.

## Rationale

Binding an active-Turn submission to the Turn the frontend actually observed
prevents scheduling and polling races from changing temporal intent. Explicit
steer, Activity-response, and queue meanings also prevent hidden backend
capability from deciding whether text corrects current work or starts later
work.
