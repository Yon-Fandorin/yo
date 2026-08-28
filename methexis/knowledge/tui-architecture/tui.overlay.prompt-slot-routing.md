---
schema: methexis.knowledge/v1alpha1
id: tui.overlay.prompt-slot-routing
kind: decision
owner: tui-architecture
sources:
  - id: tui.overlay-002
    revision: sha256:f4ae70eb71c66fa1b713e175002e9bc9314f1eb920a95032c28bec30db2327b6
relations:
  depends_on:
    - tui.overlay.selection-panel
    - tui.chrome.input-stack
  constrained_by:
    - tui.runtime.typed-flow
  applies_to:
    - yo-tui::overlay
    - yo-tui::runner::state
    - yo-tui::shell
---
# Prompt overlay slot and routing

## Statement

`TuiSession` MUST own one prompt-overlay slot that is active only while Chat is
the visible view. Completion and picker providers MUST retain their own query,
filtering, preview, and effect state and publish only validated selection-panel
snapshots into that slot. Opening a panel MUST atomically replace the prior
panel and return a non-reusable overlay instance token. Refresh, close, and
accept operations MUST present the matching token. An operation from an older
or different token MUST be rejected without changing the current slot.

Refresh MUST atomically replace the entry snapshot. It MUST preserve the
selected identity when that entry remains present and enabled. Otherwise it
MUST select the first enabled entry in stable provider order, or retain no
selection when all entries are disabled. A refresh of a previously presented
instance MUST advance a slot-owned presentation revision and enter a
presentation-pending state. A prepared frame MUST identify the exact overlay
instance token and presentation revision it evaluated. Until a frame commit
matches the current token and revision, the slot MUST consume configured
previous, next, and acceptance gestures without moving selection, issuing an
acceptance receipt, or allowing the editor to submit the draft. Ordinary editor
input and overlay dismissal MUST retain their normal routing, and a later
refresh MUST supersede every earlier uncommitted presentation revision. A stale
frame commit MUST NOT release the current fence.

A matching committed frame that visibly presents the panel MUST publish that
revision and restore normal overlay selection input. A matching committed frame
that hides the panel for insufficient geometry MUST also release the fence,
mark the slot unpresented, and make it yield first-refusal input. This
presentation synchronization belongs to the prompt slot and is distinct from
the selection panel's provider-controlled fresh or pending-replacement semantic
state. Opening an instance that has never been presented retains the existing
unpresented routing; presentation synchronization after refresh does not turn
the slot into a modal all-input owner. An unpresented or hidden instance MUST
NOT claim dismissal or consume Esc.

`accept(token)` MUST be an atomic single-consumer transition: it MUST validate
the matching current instance and enabled selection, irreversibly close that
instance, and return exactly one acceptance receipt containing the instance
token and selected opaque entry identity. Duplicate or stale acceptance MUST be
rejected. The provider owns the product effect and any failure-retry policy;
retry MUST NOT reaccept the closed panel instance.

Process termination, job-control suspend, terminal resize, global view
switching, and an agent-requested interaction MUST retain priority over the
prompt overlay. Switching away from Chat or publishing an agent-requested
interaction MUST close the slot; returning to Chat MUST NOT resurrect it. In
the presence of an outstanding interaction, automatic completion and picker
providers MUST remain blocked. A user-invoked local command palette and a local
picker reached explicitly through it MAY open a new instance only when their
own contract guarantees that no selection is encoded as the Activity response
and that selections cannot answer, cancel, or consume the Activity. One
explicitly selected process-termination command MAY be a named lifecycle
exception that ends the Session and outstanding Activity; its command draft
MUST NOT become an Activity response. Dismissing a concretely visible local
instance MUST restore ordinary response routing for the unchanged draft.

Within Chat, overlay dismiss, previous, next, and accept actions MUST be offered
to a visibly presented slot before transcript navigation, editor handling, or
active-Turn interruption. Dismiss MUST close the matching visible panel and
consume its event, so one configured `Esc` cannot also interrupt the Turn. An
instance that has never been presented or is hidden for insufficient geometry
MUST yield these actions, leaving existing chrome and active-Turn Esc semantics
unchanged. These local overlay actions MUST remain responsive while an agent
dispatch is backpressured.
`Ctrl+C` MUST be reserved from provider and overlay bindings and MUST continue
to dispatch active-Turn interruption while a panel is visible. Ordinary input
not handled by the slot MUST continue to the provider or editor, allowing
editor-attached completion to refresh after text changes.

Fitting MUST be decided before suppressing the transient work-status row. While
visibly presented, the panel destination MUST be the reserved work row plus
adjacent transcript cells, bottom-anchored directly above the prompt without
relayout. Closing the panel, or hiding it because no panel row fits, MUST
restore the work-status row from current state rather than from a captured
snapshot. The panel MUST NOT cover the prompt, the footer, or cells outside the
current frame, and opening, replacing, resizing, or closing it MUST NOT move
prompt or footer geometry. If no panel row fits, the slot MAY remain active for
state purposes but MUST yield no presentation and MUST NOT claim first-refusal
input while hidden. Modal all-input ownership, nested overlays, and overlay
stacks remain outside this revision.

## Rationale

A token-scoped single slot prevents a late asynchronous refresh from an old
provider from overwriting a newer panel. Slot-owned presentation revisions also
prevent logical selection from running ahead of the last frame the user saw
without making each provider invent a second gate. Chat-only focus prevents an
invisible panel from stealing Transcript or Request navigation. Temporarily
replacing the work-status row with the focused selection surface reduces
competing chrome, while restoration from current state avoids showing stale
activity after the panel closes.
