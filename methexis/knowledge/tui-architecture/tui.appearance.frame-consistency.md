---
schema: methexis.knowledge/v1alpha1
id: tui.appearance.frame-consistency
kind: decision
owner: tui-architecture
sources:
  - id: tui.appearance-002
    revision: sha256:8ffb2bf532c8d5e4b656352ec9cc434eb4747d0c308035593d1907ef68ac97f9
relations:
  depends_on:
    - tui.appearance.session-publication
  constrained_by:
    - tui.surface.resolved-style
  applies_to:
    - yo-tui::runner::prepare_frame
    - yo-tui::runner::session_output
    - yo-tui::components
---
# Logical-frame appearance consistency

## Statement

Logical frame preparation MUST pin one committed appearance snapshot and
revision before any component measurement. Transcript and prompt measurement,
paint, and completed `Surface` creation MUST use only that pinned snapshot. A
replacement requested while a frame is being prepared MUST affect the next
frame in full and MUST NOT partially change the current frame.

The composer MUST pass the selected resolved snapshot explicitly to each
component subtree. Components MUST NOT recover appearance through ambient or
global lookup. Presenters MUST consume the completed `Surface` and MUST NOT
perform theme or glyph resolution.

Plain `session_output` MUST use the same committed transcript configuration,
glyphs, and row layout as screen preparation; it MUST NOT independently create
a default transcript configuration. Terminal and HTML projection MUST consume
the same completed cell grid and resolved style semantics without independently
remeasuring grapheme width.

A crate-private prepared-frame seam MUST expose the pinned revision for
deterministic verification.

## Rationale

Pinning before measurement prevents mixed-width or mixed-style frames.
Explicit propagation also creates the boundary needed for a future subtree
preview while keeping screen, plain output, and projections consistent.
