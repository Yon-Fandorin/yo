---
schema: methexis.knowledge/v1alpha1
id: tui.surface.resolved-style
kind: decision
owner: tui-architecture
sources:
  - id: tui.surface-003
    revision: sha256:8f261aed5acc0e933a14062488ca9ed00f4ab65c26d77bd0947687598b24d83b
relations:
  constrained_by:
    - tui.architecture.module-boundaries
  validated_by:
    - surface.style-tests
  applies_to:
    - yo-tui::surface::style
---
# Resolved cell style

## Statement

Every physical cell MUST store its final resolved foreground, background, and
attribute `Style` inline. Semantic roles, theme lookup, and style composition
MUST happen before a write reaches `Surface`.

The initial model MUST NOT add `StyleId` indirection. It MAY replace inline
storage only after measurements show a material memory or comparison benefit
and preserve adapter-visible resolved semantics.

## Rationale

Resolved style gives diff and projection one unambiguous value to compare.
Inline storage is the smallest initial policy surface; a style table can be
introduced later without mixing theme semantics into the rendering core.
