---
schema: methexis.knowledge/v1alpha1
id: tui.appearance.activity-motion-profile
kind: decision
owner: tui-architecture
sources:
  - id: tui.motion-002
    revision: sha256:e6a023cb0c8dbad56ccf4875f601dcaffaf7b488e55a219dc03c2a358350e78c
relations:
  depends_on:
    - tui.runtime.activity-motion-scheduling
    - tui.chrome.input-stack
    - tui.overlay.selection-panel
  constrained_by:
    - tui.surface.grapheme-cells
    - tui.surface.width-profile
    - tui.appearance.frame-consistency
  applies_to:
    - yo-tui::appearance
    - yo-tui::shell::chrome
    - yo-tui::overlay::selection
---
# Built-in activity motion profile

## Statement

The initial built-in activity marker period MUST be exactly 120 milliseconds
per logical frame. The Rich profile MUST use this exact ordered ping-pong
cycle:

```text
· ✢ ✳ ✶ ✻ ✽ ✽ ✻ ✶ ✳ ✢ ·
```

The ASCII profile MUST use this exact ordered cycle:

```text
. *
```

Every frame in one profile MUST be one non-empty, control-free, renderable
extended grapheme cluster with the same cell width as every other frame in
that profile. Candidate validation MUST reject an empty frame sequence, a zero
period, an invalid frame, or unequal frame widths before publication.

The same logical frame MAY additionally move one emphasized grapheme at a time
across the visible `Working` label or a typed activity title-status published by a
selection panel. The sheen MUST change style only: it MUST preserve every
grapheme, cell width, row and panel geometry, fitting result, input behavior,
and interruption affordance. Ordinary non-busy title status MUST remain static.
The marker and every visible sheen MUST derive from the same elapsed sample.

A profile with one valid frame MUST disable marker and sheen motion and MUST
NOT arm timed redraw, leaving a future reduced-motion host choice open without
changing runner scheduling. A visible sheen MUST contain at least two
graphemes; otherwise advancing its phase cannot change a cell and MUST NOT
request timed motion.

One committed appearance snapshot and revision MUST provide the marker cycle
and period used for both frame selection and paint during a logical frame.
Replacement during preparation MUST take effect only on the next complete
frame.

## Rationale

The exact built-in sequences deliberately make the first motion behavior
reviewable while keeping cosmetic policy out of the runner. Equal-width frames
and style-only sheen ensure that animation changes cells rather than geometry, and a one-frame
candidate creates a compact reduced-motion seam without prematurely exposing
configuration. Activating this profile also selects its already-approved
`tui.appearance.frame-consistency` constraint and that unit's
`tui.appearance.session-publication` dependency; that broader eligibility
transition MUST remain explicit in the separate activation review.
