---
schema: methexis.knowledge/v1alpha1
id: tui.overlay.prompt-slot-routing
kind: decision
owner: tui-architecture
sources:
  - id: tui.overlay-002
    revision: sha256:c08b843e1358c9449f8f464fe4bf573dc080cd2256cf8bfdae5014b45d6cb50f
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
selection when all entries are disabled. `accept(token)` MUST be an atomic
single-consumer transition: it MUST validate the matching current instance and
enabled selection, irreversibly close that instance, and return exactly one
acceptance receipt containing the instance token and selected opaque entry
identity. Duplicate or stale acceptance MUST be rejected. The provider owns the
product effect and any failure-retry policy; retry MUST NOT reaccept the closed
panel instance.

Process termination, job-control suspend, terminal resize, global view
switching, and an agent-requested interaction MUST retain priority over the
prompt overlay. Switching away from Chat or publishing an agent-requested
interaction MUST close the slot; returning to Chat MUST NOT resurrect it. In
Chat, overlay dismiss, previous, next, and accept actions MUST be offered to
the slot before transcript navigation, editor handling, or active-Turn
interruption. Dismiss MUST close the matching panel and consume its event, so
one configured `Esc` cannot also interrupt the Turn. These local overlay
actions MUST remain responsive while an agent dispatch is backpressured.
`Ctrl+C` MUST be reserved from provider and overlay bindings and MUST continue
to dispatch active-Turn interruption while a panel is visible.
Ordinary input not handled by the slot MUST continue to the provider or editor,
allowing editor-attached completion to refresh after text changes.

Fitting MUST be decided before suppressing the transient work-status row. While
visibly presented, the panel destination MUST be the reserved work row plus
adjacent transcript cells, bottom-anchored directly above the prompt without
relayout. Closing the panel, or hiding it because no panel row fits, MUST
restore the work-status row from current state rather than from a captured
snapshot. The panel MUST NOT cover the prompt, the footer, or cells outside the
current frame, and opening, replacing, resizing, or closing it MUST NOT move
prompt or footer geometry. If no panel row fits, the slot MAY remain active for
state purposes but MUST yield no
presentation and MUST NOT claim first-refusal input while hidden. Modal
all-input ownership, nested overlays, and overlay stacks remain outside this
revision.

## Rationale

A token-scoped single slot prevents a late asynchronous refresh from an old
provider from overwriting a newer panel. Chat-only focus prevents an invisible
panel from stealing Transcript or Request navigation. Temporarily replacing
the work-status row with the focused selection surface reduces competing
chrome, while restoration from current state avoids showing stale activity
after the panel closes.
