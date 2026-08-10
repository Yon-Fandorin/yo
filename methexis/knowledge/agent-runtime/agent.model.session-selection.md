---
schema: methexis.knowledge/v1alpha1
id: agent.model.session-selection
kind: decision
owner: agent-runtime
sources:
  - id: agent.model-002
    revision: sha256:74dc2dd0f2f13327fd238bea14fbddcf95d170eaa36be86580bd1e838266bc8d
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

Interactive and non-interactive startup MUST accept one optional `--model MODEL_REFERENCE`. Omitting it MUST preserve a configured `model.startup`; when no startup binding is configured, omission MUST continue to start Codex. Supplying a reference for a new Session MUST be able to select a configured Yo-managed model without requiring `model.startup`.

A `MODEL_REFERENCE` has three user-facing spellings: `Model`, `Provider::Model`, and `Provider:Account:Model`. Resolution MUST compare the supplied bytes against the applicable configured catalog coordinates under all admitted spellings, deduplicate identical coordinates, and succeed only when exactly one coordinate remains. It MUST NOT let separator precedence silently choose between a ModelId and a qualified interpretation. ModelId bytes remain vendor-owned and MAY contain `:`, `/`, or `.`. Provider and Account display names never participate.

`Model` resolves inside the current Provider and Account when a startup binding or current Yo-managed binding supplies that namespace. For a new Codex-default Session with no startup namespace, it MAY search the configured catalog only for an exact ModelId and MUST require one globally unique coordinate. `Provider::Model` matches exact ProviderId and ModelId values and MUST require exactly one configured Account coordinate. `Provider:Account:Model` matches all three exact IDs. Zero matches MUST fail as absent; multiple matches MUST fail as ambiguous. Diagnostics MUST return stable, sorted, complete Provider, Account, and Model coordinates that can disambiguate the request.

In the TUI, `/model` without a value MUST open the Rib-style grouped picker for a Yo-managed binding. `/model MODEL_REFERENCE` MUST use the same frontend-neutral resolver as startup. The picker MUST group configured entries in `Provider -> Account -> Model` order and identify each row by the complete binding rather than display text or ModelId alone. Selection preparation MUST still validate credentials, tokenizer, protocol, connector, endpoint, and staleness before an effect is committed.

A resumed Yo-managed exact-replay Session MAY use the same reference grammar. Its bare `Model` form remains inside the newest durable Continuation Anchor's Provider and Account; qualified forms MAY name another configured coordinate and request the existing exact-replay replacement transition. Configured startup defaults MUST NOT replace the resume namespace. If no durable Anchor exists, the Session remains read-only under the continuation contract.

This revision does not admit cross-backend handoff. A Codex-started live Session MUST NOT expose the model picker, and a Codex resume combined with a Yo-managed model reference MUST fail explicitly until a separately reviewed transition can construct exact replay from the committed semantic boundary, record the new backend epoch and cache-loss boundary, and replace Codex-owned input providers.

A TUI selection changes only the current Yo Session. Default-model persistence remains a separate settings action. A switch MUST be prepared while the old binding remains usable, MUST be rejected during an active Turn, and MUST commit atomically as a new binding epoch. Preparation, replay, or publication failure MUST leave the old binding usable and MUST NOT create a partial epoch. Earlier messages retain the exact binding attribution that produced them.

## Rationale

One compact option keeps routine startup model-first while exact catalog matching preserves Provider, Account, and Model identity. Contextual shorthand avoids repetitive coordinates, complete references remain an explicit escape from ambiguity, and catalog-derived matching preserves vendor ModelId punctuation without inventing an escaping grammar. Separating startup selection from cross-backend replay delivers useful OpenAI-compatible startup now without pretending that Codex-managed state can already become local exact replay.
