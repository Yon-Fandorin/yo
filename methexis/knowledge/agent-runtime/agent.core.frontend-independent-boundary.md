---
schema: methexis.knowledge/v1alpha1
id: agent.core.frontend-independent-boundary
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-001
    revision: sha256:1ddc96b67f71dd2dae90856da7dd2313ef7ac339a1e390227bc36bd5b33b2292
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
when it expresses agent execution meaning. The initial monolith kept session,
command, event, configuration, and backend concerns together while those
semantic boundaries stabilized. A later crate split MUST identify a concrete
architectural boundary that changes independently rather than extracting code
only because it is shared.

The accepted backend split consists of the provider-neutral `yo-backend`
foundation, the `yo-core` `AgentBackend` semantic specialization, and flat
concrete `yo-backend-managed`, `yo-backend-delegated-codex`, and
`yo-backend-delegated-grok` crates. Concrete backends MAY depend on the
foundation and `yo-core`; `yo-core` and the foundation MUST NOT depend on a
concrete backend. A different protocol, adapter, or utility extraction still
requires an independent consumer, an independently changing host protocol or
process lifecycle, or a release boundary.

## Rationale

This boundary gives terminal and future GUI frontends one execution engine
without allowing a generic `core` name to become a miscellaneous utility
container. It also permits the reviewed backend ownership boundary without
turning every shared mechanism into a speculative crate.
