---
schema: methexis.knowledge/v1alpha1
id: tui.chrome.input-stack
kind: decision
owner: tui-architecture
sources:
  - id: tui.chrome-001
    revision: sha256:0569e99201112c9af64cbce8db9b11766dbb5c0d0938e64904c04f1cdaa27eee
relations:
  depends_on:
    - agent.runtime.active-turn-input
  constrained_by:
    - tui.surface.geometry
    - tui.surface.width-profile
  applies_to:
    - yo-tui::shell::chrome
    - yo-tui::input::control
    - yo-tui::runner::TuiSessionInfo
---
# Static input chrome stack

## Statement

The editable TUI shell MUST order its static input chrome as transient work
region, prompt, host-known metrics, then presentation mode. The transient region
MUST reserve the same geometry while idle and active whenever that region fits,
so a Turn lifecycle change changes its content rather than moving the prompt.
When the terminal is too short for every region, the shell MUST preserve the
prompt plus a readable transcript floor before optional chrome detail.

While a Turn is active, the transient region MUST expose both plain `Esc` and
`Ctrl+C` as interruption affordances and both keys MUST dispatch the same
interrupt intent. Idle `Esc` MUST remain unhandled for a future overlay owner.
Idle `Ctrl+C` MUST retain the separate clear and double-press exit policy.
Overlay-first Escape precedence requires a concrete overlay and remains a later
contract.

Status rows MUST contain only values an owning host or runtime source actually
knows. An unavailable backend, workspace, model, context, Git state, or
permission value MUST be omitted rather than inferred. Status composition MUST
use typed left and right segments with explicit priorities. On insufficient
cell width, a segment MUST be removed as a whole instead of wrapping,
truncating, or making the completed frame fail. The interruption affordances
MUST outlive decorative markers whenever both key labels still fit.

This contract owns static geometry and event-driven projection only. Spinner
frames, elapsed time, timed redraw, configurable status-line composition, and
additional status data sources require later contracts.

## Rationale

A stable prompt anchor keeps typing visually predictable while the work row
makes interruption discoverable. Honest typed segments allow the status line
to grow with real runtime data and future GUI consumers without freezing one
formatted string or presenting guesses as state.
