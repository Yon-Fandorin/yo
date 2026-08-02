---
schema: methexis.knowledge/v1alpha1
id: tui.appearance.activity-motion-profile
kind: decision
owner: tui-architecture
sources:
  - id: tui.motion-002
    revision: sha256:343e7e7b6d0ee34f9c281965e95779ba15be9eba7871d630221dbdd3ba8fe0f8
relations:
  depends_on:
    - tui.runtime.activity-motion-scheduling
    - tui.chrome.input-stack
  constrained_by:
    - tui.surface.grapheme-cells
    - tui.surface.width-profile
    - tui.appearance.frame-consistency
  applies_to:
    - yo-tui::appearance
    - yo-tui::shell::chrome
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

Motion MUST change only the decorative activity marker and MUST preserve all
independently supplied non-marker text, fitting behavior, and interruption
affordances. A profile with one valid frame MUST be representable as a
non-animated candidate and MUST NOT arm timed redraw, leaving a future
reduced-motion host choice open without changing runner scheduling.

One committed appearance snapshot and revision MUST provide the marker cycle
and period used for both frame selection and paint during a logical frame.
Replacement during preparation MUST take effect only on the next complete
frame.

## Rationale

The exact built-in sequences deliberately make the first motion behavior
reviewable while keeping cosmetic policy out of the runner. Equal-width frames
ensure that animation changes cells rather than geometry, and a one-frame
candidate creates a compact reduced-motion seam without prematurely exposing
configuration. Activating this profile also selects its already-approved
`tui.appearance.frame-consistency` constraint and that unit's
`tui.appearance.session-publication` dependency; that broader eligibility
transition MUST remain explicit in the separate activation review.
