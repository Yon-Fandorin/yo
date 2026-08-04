---
schema: methexis.knowledge/v1alpha1
id: tui.appearance.activity-motion-profile
kind: decision
owner: tui-architecture
sources:
  - id: tui.motion-002
    revision: sha256:0b58c256cc9fb7a3b4751e840846674e80b12f2fddb5d191175427cfc1ec8250
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

The built-in marker MUST use a stable appearance-resolved accent without bold
or dim attributes. Frame changes MUST NOT also change its font weight. This
avoids adding font-weight distortion to the one-cell star silhouette while
preserving the exact marker sequences above; it does not claim to control a
terminal font's own glyph overhang.

The same logical frame MAY additionally move one peak grapheme at a time
across the visible `Working` label or a typed activity title-status published
by a selection panel. Up to one adjacent grapheme on each side MUST use an
appearance-resolved intermediate trail style; the trail MUST clip at the label
edges rather than wrap. All remaining label graphemes MUST use the muted
activity style. Muted, trail, peak, and marker styles MUST remain separate
appearance roles so a profile can tune color without changing layout code.
The peak MUST advance from the first visible grapheme through the last and
then wrap to the first. One shared `ActivityMotionFrame` resolver MUST return
the peak and optional left and right trail indices for both shell chrome and
selection-panel rendering.

Built-in muted, trail, peak, and marker roles MUST use only the terminal
default foreground or palette-indexed colors. Hard-coded RGB colors are
reserved for future explicit theme configuration, where foreground and
background can be resolved together.

The sheen MUST change style only: it MUST preserve every grapheme, cell width,
row and panel geometry, fitting result, input behavior, and interruption
affordance. Ordinary non-busy title status MUST remain static. The marker and
every visible sheen MUST derive from the same elapsed sample.

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
reviewable while keeping cosmetic policy out of the runner. A stable marker
weight avoids adding font-weight distortion, while the peak and adjacent trail
turn the label into one controlled scan instead of a flashing character.
Equal-width frames and style-only sheen ensure that animation changes cells
rather than geometry, and a one-frame candidate creates a compact
reduced-motion seam without prematurely exposing configuration. Activating
this profile also selects its already-approved
`tui.appearance.frame-consistency` constraint and that unit's
`tui.appearance.session-publication` dependency; that broader eligibility
transition MUST remain explicit in the separate activation review.
