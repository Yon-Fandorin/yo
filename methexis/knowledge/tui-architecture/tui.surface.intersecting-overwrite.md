---
schema: methexis.knowledge/v1alpha1
id: tui.surface.intersecting-overwrite
kind: decision
owner: tui-architecture
sources:
  - id: tui.surface-014
    revision: sha256:9d71c6a68b137a752115546488860baf8eb68128b35b0a286dd4e64ad961d730
relations:
  depends_on:
    - tui.surface.atomic-grapheme-write
    - tui.surface.blank-cell
    - tui.surface.grapheme-cells
  validated_by:
    - surface.intersecting-overwrite-tests
  applies_to:
    - yo-tui::surface::view
---
# Intersecting grapheme overwrite

## Statement

Before a grapheme write mutates cells, it MUST compute the proposed new
footprint and the complete footprint of every existing leader or continuation
intersecting it. The atomic mutation region is their union.

If the proposed footprint or any intersecting existing footprint crosses the
current `SurfaceView` bounds, the operation MUST return `Clipped` without
mutation. Otherwise it MUST first replace the whole mutation region with
`Blank` cells using the incoming resolved style, then write the new leader and
continuations with that style as one atomic change.

This rule MUST apply when a narrower grapheme replaces a wider leader and when
a write begins on a continuation cell.

## Rationale

Closing over old footprints prevents orphaned cells and ghost text while the
view-bound check preserves component isolation. One explicit cleanup style
makes narrower and continuation-start replacement deterministic.
