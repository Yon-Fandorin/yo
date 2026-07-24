---
schema: methexis.knowledge/v1alpha1
id: tui.crate.ui-only-boundary
kind: decision
owner: tui-architecture
sources:
  - tui.arc-001
relations:
  validated_by:
    - architecture.public-api-gate
---
# UI-only crate boundary

## Statement

The first `yo-tui` production crate MUST own only UI behavior, expose a narrow
facade, and keep implementation details internally visible by default.

## Rationale

A UI-only boundary keeps application and product semantics independent from
terminal presentation while avoiding speculative crate splits.
