---
schema: methexis.knowledge/v1alpha1
id: tui.missing-target
kind: rule
owner: tui-architecture
sources:
  - tui.fixture
relations:
  depends_on:
    - tui.does-not-exist
---
## Statement

Missing targets must fail explicitly.
