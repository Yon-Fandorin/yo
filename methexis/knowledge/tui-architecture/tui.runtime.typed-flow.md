---
schema: methexis.knowledge/v1alpha1
id: tui.runtime.typed-flow
kind: decision
owner: tui-architecture
sources:
  - tui.arc-003
relations:
  depends_on:
    - tui.architecture.module-boundaries
  constrained_by:
    - tui.crate.ui-only-boundary
  validated_by:
    - runtime.backpressure-tests
    - runtime.ordering-tests
---
# Typed runtime flow

## Statement

Runtime communication MUST use named typed events and intents. Control events
use a bounded non-dropping lane, while replaceable state updates use explicit
coalescing without violating ordering or fairness.

## Rationale

Typed lanes make overload behavior reviewable and prevent important lifecycle
or approval signals from being silently discarded with replaceable UI state.
