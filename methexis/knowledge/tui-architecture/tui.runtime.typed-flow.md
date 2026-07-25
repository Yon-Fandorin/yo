---
schema: methexis.knowledge/v1alpha1
id: tui.runtime.typed-flow
kind: decision
owner: tui-architecture
sources:
  - id: tui.arc-003
    revision: sha256:33b1012a4204eaa6811533d7fd37c5015ffc9c4d97a02e394e41bab5554d3727
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
