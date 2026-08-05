---
schema: methexis.knowledge/v1alpha1
id: tui.appearance.activity-motion-profile
kind: decision
owner: tui-architecture
sources:
  - id: tui.motion-002
    revision: sha256:800df7feb6bade941beadcbf7ae1fed51d3379ebfb450ac74316eb15d99f889b
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

The initial built-in Rich activity marker MUST be the fixed one-cell grapheme
`✦` (`U+2726`). The ASCII profile MUST use the fixed marker `*` (`U+002A`).
The marker MUST NOT change shape while activity is running. This removes
font-dependent overhang changes between unlike star silhouettes and keeps one
stable cell identity across repaints.

The initial built-in animated repaint interval MUST be exactly 16 milliseconds.
An appearance candidate with an animated interval below 16 milliseconds MUST
be rejected; a slower interval remains valid for a future configured profile.
Its sweep period MUST be exactly two seconds. A late wake MUST select the
current elapsed-time phase and skip missed frames under the existing scheduling
contract; it MUST NOT replay missed frames. Adaptive cadence is deferred until
runtime evidence requires a separate scheduling policy.

For elapsed duration `e`, sweep period `T`, and a label with `N` visible
graphemes, resolution MUST use these values without first reducing `q` or `p`
to an integer:

```text
q = (e mod T) / T
p = -10 + q * (N + 20)
```

Label grapheme coordinates MUST be the zero-based integers `i = 0..N-1`.
The ten virtual positions on both sides make every label intensity zero at the
period boundary, so restarting at `q = 0` is not visible.

For each visible label grapheme, intensity MUST be zero outside a five-cell
half-width around the sweep position. Inside that band it MUST use this
raised-cosine envelope, where `d` is absolute logical distance from the sweep
position:

```text
intensity(d) = 0.5 * (1 + cos(PI * d / 5))
```

The continuous position MUST be retained until style resolution; it MUST NOT
be reduced to one integer peak index per repaint. In TrueColor mode each
grapheme MUST linearly blend between appearance-resolved base and highlight RGB
endpoints using `0.9 * intensity`. Each channel MUST use
`round(base + (highlight - base) * 0.9 * intensity)` and clamp to `0..255`.
The renderer MUST NOT hard-code those RGB endpoints. Appearance owns the
endpoints so a later terminal-palette probe or user theme can replace them
without changing shell or overlay layout code.

Before appearance publication, the process host MUST supply an explicit color
capability classified as `TrueColor`, `Limited`, or `Unknown`. The committed
appearance snapshot MUST retain that resolved value for the whole logical
frame. `Unknown` MUST follow the safe lower-depth fallback and MUST NOT emit RGB.
Capability classification is distinct from OSC palette probing: a conservative
host MAY derive it from explicit configuration or stable environment facts,
while a future lifecycle-owned probe MAY provide stronger evidence.

At lower color depths, the same intensity MUST resolve through a bounded
fallback: below `0.2` is dim, from `0.2` below `0.6` is default weight, and
`0.6` or above is bold. The fallback MUST NOT introduce RGB output. Reduced
motion MUST render a static marker and static activity label and MUST NOT arm a
timed repaint.

The fixed marker MUST use the same position and intensity equations with
`N = 1` and `i = 0`. It therefore derives a deterministic smooth pulse from the
same elapsed sample rather than changing grapheme or cell width. One shared
`ActivityMotionFrame` resolver MUST supply continuous intensity for the shell
`Working` label, the marker pulse, and typed activity title-status published by
a selection panel.

Motion MUST change style only: it MUST preserve every grapheme, cell width,
row and panel geometry, fitting result, input behavior, and interruption
affordance. Ordinary non-busy title status MUST remain static. The marker and
every visible shimmer MUST derive from the same elapsed sample.

Appearance MUST own the resolved color capability, validated marker, repaint
interval, sweep period, RGB endpoints, lower-depth fallback roles, and
reduced-motion choice. Candidate
validation MUST reject an empty, controlled, multi-grapheme, zero-width, or
over-wide marker; a repaint interval below 16 milliseconds; or a zero sweep
period before publication. Keeping these values inside the existing appearance
candidate boundary is a configuration seam, not a user-facing configuration
file in this revision.

One committed appearance snapshot and revision MUST provide the marker,
timing, endpoints, fallback, and motion mode used for both style resolution and
paint during a logical frame.
Replacement during preparation MUST take effect only on the next complete
frame.

## Rationale

The fixed marker removes the one-cell silhouette churn that appeared clipped
in some fonts. The fractional cosine sweep changes brightness gradually instead
of moving a three-level block one grapheme at a time. Off-label padding makes
the modulo reset invisible, while explicit lower-depth and reduced-motion
paths keep the behavior honest outside TrueColor terminals.

Appearance-resolved endpoints preserve the future route to terminal palette
discovery and user themes without coupling layout code to configuration. This
revision intentionally does not add OSC palette probing because that operation
shares terminal input and timeout ownership with lifecycle code and deserves a
separate contract. Activating this profile also selects its already-approved
`tui.appearance.frame-consistency` constraint and that unit's
`tui.appearance.session-publication` dependency; that broader eligibility
transition MUST remain explicit in the separate activation review.
