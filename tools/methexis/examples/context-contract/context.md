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
terminal I/O or emit raw ANSI control bytes. Terminal adapters own TTY and
terminal-output operations. The application entry host owns process-wide
lifecycle policy, including Unix signal installation and replay, and supplies
only typed control observations to repeatable UI sessions.

## Rationale

An inward dependency direction lets terminal and documentation adapters consume
one structured UI model without allowing either adapter to own component
semantics. Keeping process policy in the product entry host prevents a UI
library from becoming the lifecycle root of a future GUI or other frontend.

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
