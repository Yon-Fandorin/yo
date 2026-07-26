---
schema: methexis.knowledge/v1alpha1
id: tui.surface.geometry
kind: decision
owner: tui-architecture
sources:
  - id: tui.surface-002
    revision: sha256:db432d5ec4d0b578a05c7d1285c659a82926afc8ae9b49612ca7757096d743a6
relations:
  constrained_by:
    - tui.architecture.module-boundaries
  validated_by:
    - surface.geometry-tests
  applies_to:
    - yo-tui::surface::geometry
---
# Viewport geometry

## Statement

`Point`, `Size`, and `Rect` MUST use `u16` coordinates and dimensions. Geometry
operations MUST use checked arithmetic and report failure rather than wrap or
silently clamp. Larger document or scroll positions belong to an upper model
that maps a viewport into `Surface` coordinates.

## Rationale

Terminal viewports are bounded, but application documents are not. Keeping
those domains separate makes overflow behavior explicit and keeps the rendering
model compact.
