---
schema: methexis.knowledge/v1alpha1
id: tui.architecture.module-boundaries
kind: decision
owner: tui-architecture
sources:
  - id: tui.arc-002
    revision: sha256:a2c1cbfee356358867477b2ddd59eab6ac377d43b6461e3d6f4837d828e030c8
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
