---
schema: methexis.knowledge/v1alpha1
id: tui.chrome.input-stack
kind: decision
owner: tui-architecture
sources:
  - id: tui.chrome-001
    revision: sha256:061077b5eb85cff5badfc29f3c0de10092e5f7bdde42347894b03b649084bb38
relations:
  depends_on:
    - agent.runtime.active-turn-input
  constrained_by:
    - tui.surface.geometry
    - tui.surface.width-profile
  applies_to:
    - yo-tui::shell::chrome
    - yo-tui::input::control
    - yo-tui::input::key-notation
    - yo-tui::runner::TuiSessionInfo
---
# Static input chrome stack

## Statement

The editable TUI shell MUST order its static input chrome as transient work
region, prompt, host-known metrics, then fitted key help and presentation mode. The transient region
MUST reserve the same geometry while idle and active whenever that region fits,
so a Turn lifecycle change changes its content rather than moving the prompt.
When the terminal is too short for every region, the shell MUST preserve the
prompt plus a readable transcript floor before optional chrome detail.

While a Turn is active, the completed input stack MUST expose both plain `Esc`
and `Ctrl+C` as interruption affordances and both keys MUST dispatch the same
interrupt intent. When the footer row fits, it MUST carry those affordances;
otherwise the transient work row MUST carry them before decorative motion.
If height or width leaves neither row able to show both labels, the visual hint
MAY be omitted while both input gestures MUST continue to dispatch normally.

Input policy MUST expose semantic availability for interrupt, configured
newline, and empty-prompt exit actions. TUI presentation MUST format those
actions through one shared terminal notation owner: `Esc` for Escape, caret
form such as `^C` and `^D` for control characters, and `C-`, `M-`, or `S-`
prefixes for other modified keys. Idle `Esc` MUST NOT be advertised as an
action. A GUI MAY project the same semantic actions with platform-appropriate
labels or icons instead of terminal notation. A concrete visible prompt overlay MAY reuse the reserved
work row plus adjacent transcript cells without relayout. Fitting MUST be
decided before suppressing the ordinary work row. Its keymap-derived hints MUST
expose `Esc` close and `Ctrl+C` interrupt; `Esc` MUST dismiss only the overlay,
while `Ctrl+C` MUST bypass overlay bindings and interrupt the active Turn.
Closing the overlay, or deciding that no panel row fits, MUST restore the work
row from current state. Idle `Esc` MUST remain unhandled without a concrete
overlay owner. Idle `Ctrl+C` MUST retain the separate clear and double-press
exit policy.

Status rows MUST contain only values an owning host or runtime source actually
knows. An unavailable backend, workspace, model, context, Git state, or
permission value MUST be omitted rather than inferred. Status composition MUST
use typed left and right segments with explicit priorities. On insufficient
cell width, a segment MUST be removed as a whole instead of wrapping,
truncating, or making the completed frame fail. The interruption affordances
MUST outlive decorative markers whenever both key labels still fit.

After preserving the prompt and transcript floor, optional row allocation MUST
prefer the work row, host-known metrics, then the footer. Within a visible
active footer, interruption help MUST outlive presentation mode, configured
newline help, empty-prompt exit help, and decorative work text when those
elements cannot coexist. Newline help SHOULD outlive empty-prompt exit help.

This contract owns static geometry and event-driven projection only. Spinner
frames, elapsed time, timed redraw, configurable status-line composition, and
additional status data sources require later contracts.

## Rationale

A stable prompt anchor keeps typing visually predictable. The ordinary work row
makes interruption discoverable, while a focused overlay may carry the same
critical `Ctrl+C` affordance without competing chrome. Honest typed segments
allow the status line to grow with real runtime data and future GUI consumers
without freezing one formatted string or presenting guesses as state. A quiet
work label above the prompt and state-valid help below it separate current
activity from available control without moving the typing anchor.
