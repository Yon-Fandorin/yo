---
schema: methexis.knowledge/v1alpha1
id: tui.cycle-b
kind: rule
owner: tui-architecture
sources:
  - id: tui.fixture
    revision: sha256:3d3ff9057aadcbf2f44300bce0f97c5c84dc3c59a1a76e09eb012b299892f130
relations:
  constrained_by:
    - tui.cycle-a
---
## Statement

Cycle B is constrained by cycle A.
