---
schema: methexis.knowledge/v1alpha1
id: agent.runtime.active-turn-input
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-004
    revision: sha256:fa1cbcb15569dc696cd4015b0f5c8c50f1b08d43d709c31dd4fe27a731cf878d
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

## Nonsecret interview working copies

The initial TUI MUST support a separate editable working copy of an admitted
nonsecret interview. Its identity MUST include the original Session, Turn,
stable interview identity and captured batch revision, plus a durable UUIDv4
copy identity and an independent mutable CAS generation. Question order,
selected options, free text, notes and per-question navigation MUST retain
that revision; a changed or missing question schema MUST NOT reinterpret old
answers against a different batch. While the original Activity is outstanding,
submission MUST remain an exactly correlated Activity response under the
existing live-response admission rules.

After application restart, the TUI MUST offer recovery of the last durably
saved working copy with an explicit saved-state indication. Returning to an
already answered interview MUST create a new editable copy while retaining the
original submitted answers and Session history unchanged. Neither operation
MUST pretend to restore a dead backend RPC, resume an interrupted Turn or undo
previously executed work. Native disk resume alone is not evidence that a
pending question request still exists.

For recovered or previously submitted interviews, the TUI MUST expose an
explicit “send as a new conversation” action. Before dispatch it MUST show
an editable plain-text preview containing the declared original questions and
current answers in their captured order. Additional request context MUST be
user-entered; recovery MUST NOT automatically replay an old conversation,
provider-private payload or backend RPC identity. The explicit action MUST
start a new Session and its initial Turn through the ordinary typed command
boundary, preserving the selected backend and model and their admission
rules. It MUST NOT become an Activity response, exact-turn steer, queued Turn,
implicit resume or provider fallback. An active Turn MUST cause an explicit
busy result while preserving the working copy. Backpressure and retry MUST
retain this distinct new-Session intent and the same immutable preview.

The final preview MUST NOT exceed 64 KiB of UTF-8 text. The first excess byte
MUST prevent dispatch without truncating or deleting the working copy. A failed
or ambiguous dispatch MUST preserve the copy and MUST NOT claim successful
submission or automatically retry. A new-conversation copy may be marked submitted only from the observed
accepted initial StartTurn request. A live interview copy instead uses its exact
final Activity-response request, successful response write and observed response
Activity completion; it MUST NOT require a model accepted-request identity.
Both paths preserve the original history and distinguish submission from Turn
completion.
Reopening a submitted copy MUST remain a deliberate user action.

Secret input, storage and recovery remain deferred. A secret question or
secret-marked answer MUST NOT enter this working-copy or new-request path.
Missing, unsupported, oversized or incomplete legacy capture MUST remain
explicitly unavailable for complete interview recovery; the TUI MUST NOT infer
unseen questions from presentation or optional backend Request Audit.
