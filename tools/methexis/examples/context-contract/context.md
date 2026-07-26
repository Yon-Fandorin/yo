# Methexis Context

Canonical approved and active knowledge for this task. Treat `MUST` and `MUST NOT` as binding.

## KnowledgeUnit `tui.crate.ui-only-boundary`

# UI-only crate boundary

## Statement

The first `yo-tui` production crate MUST own only UI behavior, expose a narrow
facade, and keep implementation details internally visible by default.

## Rationale

A UI-only boundary keeps application and product semantics independent from
terminal presentation while avoiding speculative crate splits.

## KnowledgeUnit `tui.architecture.module-boundaries`

Required relations:
- constrained_by: `tui.crate.ui-only-boundary`

# Module and host boundaries

## Statement

Dependencies within `yo-tui` MUST flow toward terminal-independent foundation
modules. Components produce deterministic structured output and MUST NOT perform
terminal I/O or emit raw ANSI control bytes.

## Rationale

An inward dependency direction lets terminal and documentation adapters consume
one structured UI model without allowing either adapter to own component
semantics.

## KnowledgeUnit `tui.runtime.typed-flow`

Required relations:
- depends_on: `tui.architecture.module-boundaries`
- constrained_by: `tui.crate.ui-only-boundary`

# Typed runtime flow

## Statement

Runtime communication MUST use named typed events and intents. Control events
use a bounded non-dropping lane, while replaceable state updates use explicit
coalescing without violating ordering or fairness.

## Rationale

Typed lanes make overload behavior reviewable and prevent important lifecycle
or approval signals from being silently discarded with replaceable UI state.
