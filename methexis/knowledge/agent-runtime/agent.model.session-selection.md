---
schema: methexis.knowledge/v1alpha1
id: agent.model.session-selection
kind: decision
owner: agent-runtime
sources:
  - id: agent.model-002
    revision: sha256:967d609205f18e2a361e8b614a259eed9ce9ad9fa3392449a9f5a2b45c10606e
relations:
  depends_on:
    - agent.model.service-binding
    - agent.runtime.command-event-boundary
    - agent.session.continuation-lineage
  constrained_by:
    - tui.overlay.selection-panel
---
# Session model selection

## Statement

Interactive and non-interactive startup MUST accept `--model MODEL_ID` as an
explicit model override. In the TUI, `/model` MUST open a Rib-style selection
controller projected through the generic selection panel, and `/model
MODEL_ID` MUST provide a direct switch. The overlay remains presentation
only; a frontend-neutral controller owns catalog entries, validation,
preparation, and the accepted effect.

The picker MUST group usable entries in `Provider -> Account -> Model` order
and MUST identify each row by the complete binding rather than by display text
or ModelId alone. Provider, Account, and Model display names are labels only.
The initial catalog MUST come from validated configured entries and MUST NOT
assume that an OpenAI-compatible endpoint provides a complete account-scoped
model-list API. Remote catalog discovery and caching are deferred.

`/model MODEL_ID` MUST resolve to exactly one configured entry within the
current Provider and Account. It MUST NOT search another Account or Provider
when the ID is absent or ambiguous. Account or Provider changes require the
grouped picker. For a new Session, `--model` resolves after the configured
startup Provider and Account. For a resumed Session, it resolves within the
Provider and Account of the newest durable Continuation Anchor and requests a
replacement binding through exact replay; configured startup defaults MUST NOT
override that namespace. If no durable Anchor exists, the Session remains
read-only under the continuation contract. Ambiguity or absence in either CLI
or command resolution MUST fail explicitly rather than selecting an arbitrary
usable binding.

A TUI selection changes only the current Yo Session. Default-model persistence
belongs to ordinary settings and is a separate action. A switch MUST be
prepared and fully validated without mutating the current binding, then
committed atomically as a new binding epoch. It MUST be rejected while a Turn
is active. Preparation failure, stale selection, missing credential, unsupported
protocol, or connector startup failure MUST leave the old binding usable and
MUST NOT create a partial epoch.

Changing a Model or Account MUST NOT create a new Yo Session. Earlier messages
retain the exact binding attribution that produced them, and the replacement
binding receives only the exact semantic replay allowed by the continuation
contract.

## Rationale

Model-first UX matches commercial coding tools without exposing backend
topology. Returning an exact grouped binding prevents duplicate model names or
mutable labels from selecting the wrong credential, while transactional idle
switching preserves Session and tool-loop integrity.
