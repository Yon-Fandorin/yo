---
schema: methexis.knowledge/v1alpha1
id: tui.dependencies.selection-gate
kind: rule
owner: tui-architecture
sources:
  - tui.arc-005
relations:
  applies_to:
    - workspace.dependencies
---
# External dependency selection gate

## Statement

The first Slice that requires an external TUI dependency MUST present its
problem, alternatives, enabled feature surface, replacement boundary, and
macOS/Linux support evidence before that dependency is accepted.
