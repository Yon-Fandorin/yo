---
schema: methexis.knowledge/v1alpha1
id: tui.command-palette.module-registry
kind: decision
owner: tui-architecture
sources:
  - id: tui.command-001
    revision: sha256:bb9ca8deb3e37b8e1c860b67803ed284b35ed9a13c21eb4a9e742196752abfc8
relations:
  depends_on:
    - tui.overlay.prompt-slot-routing
    - tui.overlay.selection-panel
    - tui.chrome.input-stack
    - agent.model.session-selection
    - agent.runtime.active-turn-input
  constrained_by:
    - tui.runtime.typed-flow
  applies_to:
    - yo-tui::command
    - yo-tui::runner::state
    - yo-tui::runner::model
---
# Prompt-local command palette and module registry

## Statement

Editable Chat MUST recognize a command-palette query only when the draft has
exactly one non-whitespace token beginning with `/` and the cursor is at the
end of the complete draft. Text before that token MAY contain only whitespace;
whitespace inside or after the token, a non-whitespace prefix, or text after the
cursor MUST make the palette ineligible. The query MUST filter the stable
built-in command order by ASCII-case-insensitive invocation prefix. Version 1
MUST offer `/help`, `/model`, and `/exit` in that order. No match MUST remain a
displayable disabled state with no acceptable identity.

Each built-in command module MUST own one immutable definition containing its
stable typed identity, invocation token, concise user-facing description, and
typed local effect boundary. One registry MUST compose references to those
module-owned definitions and MUST own only their deterministic enumeration
order. At composition it MUST require unique typed identities and unique
invocation tokens and MUST fail closed before publication or parsing when
either collides. It MUST NOT duplicate command names or descriptions, implement
a command effect, or become the owner of model selection or process
termination. Palette discovery, exact built-in parsing, and the `/help` summary
MUST enumerate the same registry, so adding or changing a command cannot leave
help and discovery with different metadata. Command modules MAY delegate their
typed effect to an existing narrower owner rather than duplicate that behavior.

The palette MUST publish validated rows through the single prompt-overlay slot.
While a concrete palette is visibly presented, Up and Down MUST navigate,
Enter or Tab MUST accept, and Esc MUST close only the palette while preserving
the draft. Every eligible query revision that changes the matching snapshot,
including refinement, broadening, or replacement, MUST refresh the current slot
instance. Movement and acceptance MUST then follow the prompt slot's
presentation-synchronization boundary and cannot run against a snapshot before
a matching frame commit. Eligibility, token lifetime, global input priority,
insufficient geometry, and presentation synchronization remain governed by the
prompt-overlay contracts; the command controller MUST NOT introduce a second
interaction gate.

While the palette logically owns the current unchanged slash draft, an exact
registered invocation MUST execute that command locally even if the first
palette frame has not committed. A partial or unknown invocation submitted
without a visibly presented acceptable selection MUST be consumed locally,
MUST preserve the draft, and MUST produce a local unknown or incomplete-command
outcome available for correction. It MUST NOT infer and execute an unseen
prefix match. When a concrete palette is visibly presented, Enter or Tab MAY
instead accept its visible enabled selection. Thus `/e` MAY accept the visible
`/exit` row, but an unpresented or hidden `/e` MUST remain local and preserve
the draft. No known, partial, or unknown slash submission owned by the palette
may become an agent submission merely because frame commit or geometry timing
changed.

Only Esc that dismisses a concrete visibly presented palette MUST arm the exact
unchanged draft for one ordinary submission that bypasses command parsing and
follows the current prompt owner. An instance that has never been presented or
is hidden for insufficient geometry MUST NOT claim Esc; the existing chrome
and active-Turn input rules continue to own it. Any edit MUST clear the visible
dismissal escape and make the resulting draft eligible for normal palette
scanning again; the bypass also ends after that one submission. Successful
local effect admission or delegated picker admission MUST consume and clear the
exact command draft. A local validation or admission failure before the effect
starts MUST preserve the draft for correction and report the failure locally.

An outstanding approval or agent-requested input MUST remain pending while the
user invokes the palette. `/help` and `/model` MUST NOT answer, cancel, or
otherwise consume that Activity. If the user dismisses a visible palette with
Esc and submits the unchanged draft, the Activity response owner MUST receive
it through the ordinary response path. `/help` MUST append a local summary
generated from the registry and return focus to the still-pending response.
`/model` MUST delegate to the session model-selection flow: idle selection may
switch immediately, while selection during an active Turn may reserve the
chosen model for the next Turn under that owner's rules. `/exit` MUST return
the existing runner exit effect and is the deliberate process-lifecycle
exception: it MAY terminate or interrupt the Session and its outstanding
Activity, but it MUST NOT encode the command draft as an Activity response.
Argument-bearing model selection remains owned by the model-selection flow and
is not a palette entry.

## Rationale

Module-owned definitions keep behavior near its natural owner, while a shallow
registry gives discovery and help one deterministic composition point without
turning it into a command god object. Logical draft ownership keeps slash text
out of agent dispatch across frame timing, while visible dismissal makes the
exception an action the user could actually observe. Local commands can remain
available without stealing an outstanding Activity response, with process exit
named honestly as a lifecycle exception. Reusing the prompt-overlay slot
preserves the established input and geometry boundary, and shared presentation
synchronization prevents a fast key sequence from executing a choice the user
has not yet seen.
