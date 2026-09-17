---
schema: methexis.knowledge/v1alpha1
id: agent.runtime.active-turn-input
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-004
    revision: sha256:ee4f351128f49d0f86e9f4e43bf9e51d5519de90115069bc2acf840a2bdf96e0
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

Missing, unsupported, oversized or incomplete legacy capture MUST remain
explicitly unavailable for complete interview recovery; the TUI MUST NOT infer
unseen questions from presentation or optional backend Request Audit.

## Live secret interview answers

A secret answer MAY be admitted only for a current, exactly correlated Activity
question whose typed backend-neutral presentation explicitly marks it secret.
The answer's content MUST NOT be inspected to infer secrecy. This first profile
admits only free-text secret questions with no choices and no notes. Unsupported,
malformed or oversized secret presentation MUST fail closed before answer input
is enabled and MUST NOT fall back to the ordinary plain-text editor. A frontend
MUST wait for the request presentation before accepting question input, even
when it has already observed the enclosing UserInputRequest Activity start.

The TUI MUST collect a secret through a request-bound editor that does not use
ordinary prompt history, kill/yank state, command or skill expansion, workspace
references, attachments, notes, working-copy edits or preview text. It MUST
render only a fixed public state such as `Not entered`, `Entered` or
`Re-entry required`; rendering, cursor geometry and receipts MUST NOT reveal the
answer or its length. Committed Unicode text and bracketed paste are literal
answer bytes. Embedded newlines, slash-prefixed text, `@` and `$` text MUST NOT
submit or activate another input path. Only an explicit submit key press for the
currently presented request may submit. Leaving an unsubmitted secret question,
cancelling the request, completing or interrupting its Turn, or replacing its
request MUST discard the value. Returning to a secret question requires fresh
entry. An individual secret is limited to 64 KiB of UTF-8 and all live secrets
retained for one batch are limited to 256 KiB; the first excess byte rejects the
input without truncation.

The live response MUST carry the exact answer only to the backend that owns the
same outstanding ActivityRequestRef. Before backend dispatch, existing bounded
backpressure MAY retain that same request-bound intent. Once transport write is
attempted, a failure or disconnect has an unknown delivery outcome: Yo MUST
discard the value, MUST NOT automatically retry it and MUST reject another
response attempt for that request. A successful backend command, including one that only stages an intermediate
answer, permits only a payload-free semantic receipt to cross the command-commit
and Journal boundary; only the final aggregate response write and final seal
establish batch submission.
For any batch containing secret questions, including an all-secret batch,
earlier secret answers MAY remain process-local until the one final batch
response is written; they MUST be discarded on final success,
failure, cancellation or Turn termination.

Restart and interview recovery restore public questions, public answers and
submission evidence only. Each secret answer is restored as `Re-entry required`,
without a value, hash or length, and can be entered again only for a new genuine
live secret request. A working copy containing a secret question MUST reject the
existing `send as a new conversation` action because that path is ordinary
persisted StartTurn input and has no original outstanding Activity. It MUST NOT
send an empty answer, a mask or a redaction label as a substitute.

Yo MUST NOT directly copy the entered secret value into its input display,
scrollback, Journal, interview working copy, preview, export, logs or
diagnostics. Delivery to the requesting backend is intentional. Backend or
provider retention, later model or tool output, the system clipboard, swap,
process memory and crash dumps are outside this guarantee. Secret interview
input is not credential storage and does not add OS keychain behavior.
