---
schema: methexis.knowledge/v1alpha1
id: agent.core.frontend-independent-boundary
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-001
    revision: sha256:40a0bbfa90eee27e96941b38090a22f368967cd45071910f3303ced1aabc02e3
relations:
  constrained_by:
    - tui.architecture.module-boundaries
    - tui.crate.ui-only-boundary
---
# Frontend-independent agent core

## Statement

The shared agent engine MUST be named `yo-core` and own only
frontend-independent agent execution semantics. `yo-tui` owns UI behavior,
`yo-cli` owns the product entry point, process-wide lifecycle policy, and
top-level wiring, and a future GUI MUST reuse `yo-core` without depending on
`yo-tui`.

Shared use alone MUST NOT qualify code for `yo-core`; code belongs there only
when it expresses agent execution meaning. The initial implementation MUST keep
session, command, event, configuration, and backend concerns as internal
modules of one crate. A separate protocol or adapter crate requires an
independent consumer or release boundary.

## Rationale

This boundary gives terminal and future GUI frontends one execution engine
without allowing a generic `core` name to become a miscellaneous utility
container or causing speculative crate splits.
