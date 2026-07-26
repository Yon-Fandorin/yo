---
schema: methexis.knowledge/v1alpha1
id: tui.surface.html-projection
kind: decision
owner: tui-architecture
sources:
  - id: tui.surface-010
    revision: sha256:869b76d54d7f56600fd503d70309df0e1f649d9c0c0a1f109d9344d8eed54d95
relations:
  depends_on:
    - tui.surface.grapheme-cells
    - tui.surface.model-ownership
  constrained_by:
    - tui.architecture.module-boundaries
  validated_by:
    - html.surface-parity-fixtures
  applies_to:
    - yo-tui::html
---
# Direct HTML projection

## Statement

The initial HTML adapter MUST deterministically project a completed `Surface`
directly into an HTML/CSS fragment using the same grapheme occupancy, width
profile, and resolved styles as terminal rendering. The canonical fragment MUST
be separable from optional developer viewer or inspector chrome.

The initial adapter MUST NOT emulate ANSI or browser terminal reflow. A replay
adapter MAY be added later if actual debugging evidence requires operation-level
inspection.

## Rationale

Direct state projection gives agents a stable, inspectable representation and
supports a future web UI without treating browser behavior as terminal truth.
