---
schema: methexis.knowledge/v1alpha1
id: tui.surface.bounded-view
kind: decision
owner: tui-architecture
sources:
  - id: tui.surface-006
    revision: sha256:281d917d4c79bb359405194e98ee9c4f10cd8da610fd00f7becd91e0d64c796c
relations:
  depends_on:
    - tui.surface.grapheme-cells
    - tui.surface.model-ownership
  constrained_by:
    - tui.crate.ui-only-boundary
  validated_by:
    - surface.view-boundary-tests
  applies_to:
    - yo-tui::surface::view
---
# Bounded component view

## Statement

Components MUST render through a `SurfaceView` bounded to an assigned `Rect`.
The view MUST expose readable final cell state and a small set of validated
write operations, but MUST NOT expose mutable backing storage.

The initial renderer MUST NOT add retained widget trees, layers, or z-index.
The caller determines composition by invoking component renders in an explicit
order.

## Rationale

A narrow view prevents a component from corrupting unrelated regions while
keeping composition predictable. Explicit draw order is sufficient until real
overlap requirements justify another model.
