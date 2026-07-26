---
schema: methexis.knowledge/v1alpha1
id: tui.surface.atomic-grapheme-write
kind: decision
owner: tui-architecture
sources:
  - id: tui.surface-007
    revision: sha256:7b765f36083f502eeafc68aa00492202a50c50261d5280c36f7fb24984928c32
relations:
  depends_on:
    - tui.surface.bounded-view
    - tui.surface.grapheme-cells
  validated_by:
    - surface.atomic-write-tests
  applies_to:
    - yo-tui::surface::view
---
# Atomic grapheme write

## Statement

A grapheme write MUST either update every physical cell it occupies or make no
mutation. If the complete grapheme does not fit in the remaining view bounds,
the operation MUST return `Clipped` and leave prior state unchanged.

The primitive owns physical cell overwrite and complete footprint cleanup
without shifting or compacting surrounding cells. Wrapping, ellipsis, logical
text-sequence insertion, deletion, replacement, and width-changing reflow
belong to the text model and layout above `SurfaceView`. That layer MUST
recompute and render final positions, so logically replacing `가B` with `AB`
produces adjacent `A` and `B` cells rather than exposing the cleared
continuation as a visible gap.

## Rationale

Atomic failure prevents orphaned continuation cells and hidden partial writes.
Keeping reflow above the cell primitive also prevents a local write from
shifting a border or another component that merely occupies an adjacent cell.
