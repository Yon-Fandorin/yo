---
schema: methexis.knowledge/v1alpha1
id: agent.runtime.command-event-boundary
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-002
    revision: sha256:b2671677903e387af78072c5ce520e250013ab6bf9d2a6d7a8f54471686baa38
relations:
  depends_on:
    - agent.core.frontend-independent-boundary
    - agent.runtime.session-turn-activity
    - tui.runtime.typed-flow
---
# Agent command and event boundary

## Statement

Frontends MUST interact with `yo-core` through named typed commands and events
instead of manipulating Session or Agent Backend internals. Commands and events
MUST identify the Session and, when applicable, the Turn they affect. Whenever an
Activity or request-correlation target is applicable, including Activity
creation, start, update, and response paths, the command or event MUST carry
that Activity identity or explicit request-correlation identity. `yo-core`
decides execution behavior and emits semantic observations; a frontend decides
input gestures and presentation. A command is the agent-domain intent carried
by the existing typed runtime flow.

The initial boundary MAY use in-process Rust types and channels. It MUST NOT
encode Codex-specific wire names as yo domain types or introduce a remote wire
protocol before a real remote consumer exists.

## Rationale

One semantic boundary lets TUI and future GUI clients share behavior while
keeping backend protocols, rendering, and input policy independently
replaceable.
