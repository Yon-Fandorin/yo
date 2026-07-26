---
schema: methexis.knowledge/v1alpha1
id: tui.surface.terminal-ops
kind: decision
owner: tui-architecture
sources:
  - id: tui.surface-009
    revision: sha256:f6030cde4a874b81529ee571d8b6a2a2aeba05d31f1a7faaaa4e394439394315
relations:
  depends_on:
    - tui.runtime.typed-flow
    - tui.surface.deterministic-diff
  constrained_by:
    - tui.architecture.module-boundaries
  validated_by:
    - terminal.op-fixtures
    - terminal.mode-restoration-tests
  applies_to:
    - yo-tui::terminal
---
# Typed terminal operation boundary

## Statement

Terminal rendering MUST follow `FrameDiff -> TerminalOp -> ANSI encoder`.
`TerminalOp` MUST represent effects such as cursor movement, resolved style
selection, and grapheme writes as typed values before byte encoding.

Inline and Fullscreen modes MUST share `Surface`, diff, and terminal operation
semantics. An outer mode controller MUST exclusively own terminal entry,
restoration, and cursor policy.

## Rationale

A typed intermediate boundary makes ordering testable without a live terminal.
Keeping mode lifecycle outside rendering prevents terminal side effects from
leaking into reusable UI state.
