---
schema: methexis.knowledge/v1alpha1
id: tui.architecture.module-boundaries
kind: decision
owner: tui-architecture
sources:
  - id: tui.arc-002
    revision: sha256:a7bfae9c0bad769321988e9d2d075341ff173177aa22a40fad9ea58b44dfd155
relations:
  constrained_by:
    - tui.crate.ui-only-boundary
  validated_by:
    - architecture.import-gate
    - architecture.raw-ansi-gate
---
# Module and host boundaries

## Statement

Dependencies within `yo-tui` MUST flow toward terminal-independent foundation
modules. Components produce deterministic structured output and MUST NOT perform
terminal I/O or emit raw ANSI control bytes. Terminal adapters own TTY and
terminal-output operations. The application entry host owns process-wide
lifecycle policy, including Unix signal installation and replay, and supplies
only typed control observations to repeatable UI sessions.

## Rationale

An inward dependency direction lets terminal and documentation adapters consume
one structured UI model without allowing either adapter to own component
semantics. Keeping process policy in the product entry host prevents a UI
library from becoming the lifecycle root of a future GUI or other frontend.
