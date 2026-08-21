---
schema: methexis.knowledge/v1alpha1
id: agent.core.frontend-independent-boundary
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-001
    revision: sha256:90d8ac42eb1a12ea40ca69726c4839c4294fab551e437b64d34705e600902a89
relations:
  constrained_by:
    - tui.architecture.module-boundaries
    - tui.crate.ui-only-boundary
---
# Frontend-independent agent core

## Statement

The shared agent engine MUST be named `yo-core` and own only frontend-independent agent execution semantics. `yo-tui` owns UI behavior, `yo-cli` owns the product entry point, process-wide lifecycle policy, top-level wiring, and concrete Backend and Model Connector composition, and a future GUI MUST reuse `yo-core` without depending on `yo-tui`.

Shared use alone MUST NOT qualify code for `yo-core`; code belongs there only when it expresses agent execution meaning. The initial monolith kept session, command, event, configuration, Backend, and Model Connector concerns together while those semantic boundaries stabilized. A later crate split MUST identify a concrete architectural boundary that changes independently rather than extracting code only because it is shared.

The accepted Backend split consists of the provider-neutral `yo-backend` foundation, the `yo-core` `AgentBackend` semantic specialization, and flat concrete `yo-backend-managed`, `yo-backend-delegated-codex`, and `yo-backend-delegated-grok` crates. The foundation retains generic evidence and replay types, including only a bounded versioned opaque provider-private envelope without interpreting it. Concrete backends MAY depend on the foundation and `yo-core`; `yo-core` and the foundation MUST NOT depend on a concrete backend.

The accepted Model Connector split keeps only the provider-neutral Connector port and shared Connector semantic request, observation, failure, cancellation, and complete-binding types in `yo-core`, including the closed registry that derives one exact Connector identity from an admitted `api_dialect` and complete binding without Provider probing or fallback. Flat concrete `yo-connector-openai-responses`, `yo-connector-openai-chat-completions`, and `yo-connector-kimi` crates MUST depend on `yo-core`, MAY depend on `yo-backend` only for its neutral replay contract and opaque envelope, and MUST NOT depend on one another. `yo-core` MUST NOT depend on a concrete Connector. `yo-cli` owns their process-wide construction and injection. A narrow `yo-connector-transport` helper is admitted only because multiple independently changing concrete Connectors share bounded HTTPS and SSE byte lifecycle mechanics; it MUST NOT become an API dialect, Provider policy, semantic replay meaning, or provider-private interpretation owner.

A different protocol, adapter, or utility extraction still requires an independent consumer, an independently changing host or service protocol or process lifecycle, or a release boundary. A directory shape, shared dependency, or wish to reduce one file's size is not sufficient.

## Rationale

This boundary gives terminal and future GUI frontends one execution engine without allowing a generic `core` name to become a miscellaneous utility or Provider implementation container. It permits the reviewed Backend and Model Connector ownership boundaries while retaining dependency inversion: the semantic core defines ports, the neutral backend foundation owns replay correlation and bounds, independently changing adapters interpret their own wire formats, and the product composition root selects exact implementations. Restricting the shared transport helper to byte lifecycle mechanics avoids both duplicated cancellation and cleanup code and a second hidden semantic core.
