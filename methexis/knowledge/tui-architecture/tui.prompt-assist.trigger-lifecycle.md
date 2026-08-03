---
schema: methexis.knowledge/v1alpha1
id: tui.prompt-assist.trigger-lifecycle
kind: decision
owner: tui-architecture
sources:
  - id: tui.assist-001
    revision: sha256:2abc8454671150a52ffe2348c63cb1ed8e24205e3281d3536d9d305094e04d07
relations:
  depends_on:
    - tui.overlay.prompt-slot-routing
    - tui.overlay.selection-panel
    - tui.chrome.input-stack
    - agent.input.workspace-reference
    - agent.input.explicit-skill-reference
  constrained_by:
    - tui.runtime.typed-flow
  applies_to:
    - yo-tui::input::editor
    - yo-tui::overlay
    - yo-tui::runner::state
---
# Prompt-assist trigger lifecycle

## Statement

One pure trigger scanner MUST inspect the editable Chat draft and cursor and
return at most one cursor-local active trigger. The supported trigger kinds are
workspace reference (`@`) and explicit skill (`$`). A trigger is eligible only
at draft start or immediately after Unicode whitespace, and its query extends
through the complete non-whitespace token containing the cursor. Mid-token
forms such as email addresses and identifiers MUST remain ordinary text.
Manually typed text MUST NOT acquire reference meaning merely because it has
the same spelling as a candidate. Before opening assistance, the scanner MUST
reject any proposed raw trigger span that equals or intersects an intact
accepted annotation, including when the cursor is at its trailing boundary. An
accepted annotation MAY span whitespace in a
workspace path even though a raw trigger query cannot; its half-open span and
opaque identity, rather than token grammar, delimit it.

One prompt-assist controller MUST own the editor revision, trigger instance,
exact replacement span, provider request identity, and accepted typed-reference
annotations. Ordinary typing,
paste, deletion, and cursor movement MUST continue through the editor and then
rescan the resulting draft. A new trigger kind or instance MUST atomically
replace the prior prompt overlay. `Esc` MUST close only the current menu and
preserve the draft; a later edit MUST be eligible to scan and open again without
a persistent suppression or escape grammar. Consequently, literal eligible
tokens such as `$HOME` may reopen assistance after a later edit in version 1;
the user can dismiss the current menu without altering the text.

Provider discovery MUST remain outside `yo-tui`. A request and every partial or
final result MUST carry one opaque assist-request identity and match an immutable
draft snapshot containing the text revision, cursor, replacement span, and
expected trigger text. Updates within one request MUST have a monotonic sequence
and exactly one terminal state. The controller MUST reject out-of-order updates
and every update after final or cancellation. A stale or cancelled result MUST
NOT refresh a newer overlay or edit the draft. Search work MUST remain
independent of agent-command backpressure, and superseded queued queries SHOULD
coalesce to the newest revision.

While a concrete enabled result is visibly presented, `Tab` or `Enter` MUST
accept it and MUST NOT submit the draft. Submission requires a later `Enter`.
Acceptance MUST atomically replace the complete trigger token, preserve all
surrounding text, close that overlay instance, and attach the returned opaque
identity to the inserted visible token. It MUST fail without mutation when the
request identity, snapshot, span, or expected trigger text is stale. Accepted
annotations use half-open spans. An edit before or after a span, including an
insertion exactly at either boundary, MUST shift or preserve the annotation;
an insertion strictly inside it or replacement/deletion intersecting it MUST
remove its typed meaning while preserving the resulting visible text as
ordinary draft content.

Disabled status, loading, no-match, and error rows MAY be presented but MUST
never be accepted. Global lifecycle and view routing, agent-requested
interaction, hidden-panel behavior, `Esc`, and `Ctrl+C` priority remain governed
by the prompt-slot contract. Prompt assistance MUST NOT create a second overlay
stack or claim input while its panel is hidden. `yo-tui` owns gesture scanning,
menu presentation, and editor-span transforms only; frontend-neutral provider
requests, typed references, and admission results cross the existing `yo-core`
command/event boundary. `yo-cli` wires the local process but MUST NOT own
provider semantics.

## Rationale

A single scanner prevents `@` and `$` from developing incompatible boundary and
replacement rules. Revision-checked annotations preserve exact semantic
identity for remote execution, duplicate names, session persistence, and a
future GUI without turning raw user text into an accidental command.
