---
schema: methexis.knowledge/v1alpha1
id: tui.dependencies.selection-gate
kind: rule
owner: tui-architecture
sources:
  - id: tui.arc-005
    revision: sha256:f0a837e8f5083f84eca42901a671f4771d061634f31da1ffab32ff963fbddef2
relations:
  applies_to:
    - workspace.dependencies
---
# External dependency selection gate

## Statement

The first Slice that requires an external TUI dependency MUST present its
problem, alternatives, enabled feature surface, replacement boundary, and
macOS/Linux support evidence before that dependency is accepted.
