---
schema: methexis.knowledge/v1alpha1
id: tui.architecture.module-boundaries
kind: decision
owner: tui-architecture
sources:
  - tui.arc-002
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
terminal I/O or emit raw ANSI control bytes.

## Rationale

An inward dependency direction lets terminal and documentation adapters consume
one structured UI model without allowing either adapter to own component
semantics.
