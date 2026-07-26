---
schema: methexis.knowledge/v1alpha1
id: tui.surface.model-ownership
kind: decision
owner: tui-architecture
sources:
  - id: tui.surface-001
    revision: sha256:664bce95595fcb8557fd9168c90714ec011446aaa0e906da36a89cd42294aa05
relations:
  depends_on:
    - tui.surface.blank-cell
    - tui.surface.geometry
    - tui.surface.resolved-style
  constrained_by:
    - tui.architecture.module-boundaries
    - tui.crate.ui-only-boundary
  validated_by:
    - surface.model-tests
  applies_to:
    - yo-tui::surface
---
# Surface model ownership

## Statement

`Surface` MUST own deterministic completed two-dimensional cell state. Terminal
entry, restoration, cursor policy, I/O, and logical scroll positions larger
than the viewport MUST remain outside the model.

## Rationale

A completed frame is reusable by terminal and HTML adapters, while lifecycle
and application state vary by environment and mode.
