---
schema: methexis.knowledge/v1alpha1
id: tui.surface.grapheme-cells
kind: decision
owner: tui-architecture
sources:
  - id: tui.surface-004
    revision: sha256:c2c4a6c44f6fa03c598df0aef1afe0fc36dd4ebfb686bcac53a8c19215f52ea8
relations:
  depends_on:
    - tui.surface.model-ownership
    - tui.surface.resolved-style
    - tui.surface.text-segmentation
    - tui.surface.width-profile
  validated_by:
    - surface.grapheme-invariant-tests
  applies_to:
    - yo-tui::surface::cell
---
# Grapheme cell representation

## Statement

A rendered grapheme MUST have exactly one leader cell that owns its complete
grapheme string and display width. Each occupied trailing cell MUST be a
continuation containing a nonzero backward distance to that leader. Every
leader and continuation cell MUST also contain the final resolved `Style` for
its physical position.

Mutation MUST preserve the invariant that no continuation is orphaned and no
leader claims cells outside the `Surface`.

## Rationale

Explicit occupancy makes wide-character overwrite and diff behavior
deterministic. A relative back-reference lets adapters and diagnostics recover
the owner without copying grapheme text into every cell.
