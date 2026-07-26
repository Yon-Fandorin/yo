---
schema: methexis.knowledge/v1alpha1
id: tui.context.base
kind: rule
owner: tui-architecture
sources:
  - id: tui.fixture
    revision: sha256:3d3ff9057aadcbf2f44300bce0f97c5c84dc3c59a1a76e09eb012b299892f130
relations:
  applies_to:
    - src/base.rs
    - yo::base
---
# Context base

## Statement

The base constraint MUST be included before every dependent consumer.
