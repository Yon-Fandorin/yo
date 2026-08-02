---
schema: methexis.knowledge/v1alpha1
id: tui.runtime.activity-motion-scheduling
kind: decision
owner: tui-architecture
sources:
  - id: tui.motion-001
    revision: sha256:6ec2c20974fa89e63ebae65bb310fb2d065884e80f24db14c7abf9765affbab4
relations:
  depends_on:
    - tui.chrome.input-stack
  constrained_by:
    - tui.terminal.lifecycle-restoration
    - tui.surface.deterministic-diff
  applies_to:
    - yo-tui::runner::unix
    - yo-tui::runner::PreparedFrame
    - yo-tui::shell::chrome
---
# Demand-driven activity motion scheduling

## Statement

Each live terminal ownership generation MUST create one monotonic animation
epoch owned by the live runner. State, components, completed Surfaces, terminal
presenters, and HTML projection MUST NOT read a clock, sleep, or create an
animation thread. Frame preparation MUST receive an explicit elapsed sample
derived from that epoch.

A prepared frame MUST report typed motion demand only when it actually painted
a dynamic activity marker. An active semantic Turn alone MUST NOT arm timed
redraw: a hidden marker caused by another view, insufficient height, or width
fallback MUST remain free of invisible animation work. Idle frames and
zero-sized surfaces MUST disarm timed redraw.

For a demanded positive frame period `P`, the logical tick MUST be
`floor((now - epoch) / P)` and the next deadline MUST be
`epoch + (tick + 1) * P`. A late wake MUST select the current logical tick and
skip every missed tick; it MUST NOT issue catch-up redraws. Event-driven redraw
and a due animation redraw MUST coalesce into one preparation using the current
sample. Every input wait path, including dispatch backpressure, MUST include an
armed animation deadline without weakening input, termination, or retry
responsiveness.

Zero-size intervals MUST preserve the generation epoch while suppressing
render work. The first later visible frame MUST use the then-current logical
tick. Suspend and resume MUST preserve retained semantic Turn state but MUST
create a fresh animation epoch with the fresh terminal generation. Both inline
and fullscreen modes MUST use the same prepared-Surface and presenter path for
event and animation redraws.

Terminal-independent frame preparation MAY receive an explicitly supplied
elapsed sample. Presenters and HTML projection MUST consume only the resulting
completed Surface and MUST NOT autonomously advance motion. Archival output
MUST remain outside activity-chrome projection.

## Rationale

Demand returned by the completed frame keeps scheduling independent of layout
internals while avoiding remote-terminal traffic for motion no user can see.
Deadline-derived ticks are deterministic under slow rendering and prevent a
stalled process from replaying obsolete animation work.
