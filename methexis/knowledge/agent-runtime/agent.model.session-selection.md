---
schema: methexis.knowledge/v1alpha1
id: agent.model.session-selection
kind: decision
owner: agent-runtime
sources:
  - id: agent.model-002
    revision: sha256:9837f286a472af60685763a53939ecdf20adde6ac87e2bafb79e6e33f65aa729
relations:
  depends_on:
    - agent.model.service-binding
    - agent.runtime.command-event-boundary
    - agent.session.continuation-lineage
  constrained_by:
    - tui.overlay.selection-panel
---
# Startup target and Session model selection

## Statement

A startup target is HostTarget or ModelTarget. Exact `host:codex` is the first HostTarget and is displayed as Local Codex. A new managed ProviderId `host` is reserved; a pre-existing manual or durable `host` coordinate remains a qualified ModelTarget with stable credential, attribution, and continuation identity and is never shadowed by the HostTarget. Interactive and non-interactive startup accept one optional `--model TARGET_REFERENCE`.

ModelTarget spellings are `Model`, `Provider::Model`, and `Provider:Account:Model`. Provider and Account canonical percent encoding maps `%` to `%25` and `:` to `%3A`; lowercase, unnecessary, malformed, or non-UTF-8 escapes fail. The Model suffix remains vendor-owned bytes. The resolver compares canonical catalog spellings, deduplicates identical coordinates, and requires exactly one result. Bare exact `host:codex` is the HostTarget; a ModelId with the same bytes requires qualification. Display names never route.

Bare Model uses the current Provider and Account namespace when one exists, otherwise global exact uniqueness. `Provider::Model` requires exactly one Account. The full spelling decodes its first two segments and addresses the complete coordinate. Zero results are absent and multiple results ambiguous; diagnostics return stable sorted canonical complete coordinates.

Startup captures four selectable sources exactly once: the invocation layer is the optional parsed `--model`; the stored layer is the three-state preference in the captured ConnectionRepository snapshot; the injected PolicySnapshot contains admission rules, `allow_user_override`, an optional enforced target, and an optional policy-default target; and the operator layer is optional `model.startup` from the command-local read-only `config.yaml`. Absence at each layer is an explicit value, not a parse failure. PolicySnapshot has exactly two valid forms. The overridable form has `allow_user_override=true`, no enforced target, and an optional policy-default target. The enforced form has `allow_user_override=false`, exactly one enforced target, and no policy-default target. Every other field combination is malformed policy. The initially shipped policy is overridable with no policy-default target and admits Local Codex plus structurally valid configured ModelTargets.

Capturing or structurally decoding any required source is fatal, as are stale repository revisions, unequal manual and managed identities at one coordinate, and malformed policy. These assembly failures are not hidden by a higher target. Enforced form selects its enforced target; a differing invocation target is a fatal policy conflict, while stored and operator targets remain visible only as non-selecting provenance. Overridable form chooses the first present invocation, stored preference, policy default, then operator `model.startup`. There is no implicit target and `host:codex` is never silently inserted. When all selectable sources are absent, interactive startup enters setup before creating a Yo Session or backend epoch; non-interactive startup fails `StartupTargetRequired` and shows exact `yo connect` and `--model host:codex` guidance. After a target is selected, missing, stale, unavailable, unsupported, or policy-denied status is fatal with no fallback.

This unit alone owns the stored user preference: unset, HostTarget, or ModelTarget. `yo default TARGET` persists one admitted choice and `--unset` clears it. Clearing may make the next startup enter setup when no policy or operator target exists. The interactive picker offers inherited, Local Codex, and complete configured models; a non-interactive value-less command fails.

`yo connect` without a target opens onboarding before Session creation. It offers Local Codex and exactly configured or newly entered external model choices without treating either as an implicit default. Selecting Local Codex verifies that the local Codex backend and its stable host identity are available and then prepares a HostTarget preference mutation; selecting an external model completes the credential, endpoint, dialect, entitlement, and semantic terminal verification owned by the service-binding contract and prepares its ModelTarget plus managed binding mutation. A non-interactive connect requires one exact target.

The first successfully verified `yo connect` whose captured stored preference is unset writes that exact HostTarget or ModelTarget as the preference in the same ConnectionRepository CAS as its successful connection outcome. A failed or cancelled attempt writes no preference. Later successful connections preserve an existing preference; changing it requires `yo default` or the explicit default-selection UI. Concurrent first connections race on the same public revision: one exact CAS wins, while a loser re-reads the winner's preference and must not replace it implicitly.

Before disconnect removes targets, selection derives one prospective transition. If the exact explicit ModelTarget preference is removed, the same public CAS clears it; otherwise it is preserved. Preview shows the old value, transition, and effective lower target or setup-required outcome. HostTarget is never cleared by model removal.

`yo model` and `/model` are ModelTarget-only. Preparation validates policy, credential, tokenizer, protocol, connector, endpoint, profile digest, and staleness. A live TUI switch prepares while the old binding remains usable, rejects an active Turn, atomically closes the old epoch and opens the new one, and leaves the old binding usable on preparation, replay, or publication failure.

Resume selects the newest durable Continuation Anchor before considering an override and never consults stored preference, policy default, or operator `model.startup`. With no explicit override it uses the Anchor binding subject to current policy and exact credential availability. Policy denial or missing credential opens history read-only with denial or reconnect guidance and no fallback, Anchor mutation, or epoch. A HostTarget override that identifies the same Codex backend is a same-binding confirmation; an exact override equal to the Anchor binding does not create a replacement epoch.

An explicit override naming a different binding is not a startup substitution. Replacement requires both a target that supports exact semantic replay and source-Anchor evidence for an admissible exact replay chain under the continuation-lineage contract, or a separately reviewed provider export that establishes the same semantic boundary and replay-content and contract digests. A backend-managed-state locator or target capability alone is never replay evidence, and Transcript or Request Audit data cannot be used to synthesize it. Without admissible source evidence, the saved Session remains read-only and the override fails with exact-replay-unavailable guidance; it may offer only the separately approved lossy transition. This rule applies to every backend-managed or local source, not only Codex.

When eligible, the replacement-binding transition owned by the Session continuation-lineage contract binds the source epoch and Anchor, fully committed semantic boundary, target complete binding identity, replay executor, replay-content and contract digests, known cache-loss boundary, and new epoch identity. Replay preparation must complete while the old epoch and Anchor remain unchanged. One atomic durable Journal transition then closes the source epoch, opens the replacement epoch, and publishes its Continuation lineage. Failure leaves the original Anchor and epoch executable when its recorded strategy still works, otherwise it opens the saved Session read-only; it never publishes a partial replacement.

A backend-managed-state binding reconnects only through its recorded locator and verified backend identity. A different binding cannot reuse that locator. Cross-backend handoff remains deferred: a Codex live Session has no ModelTarget picker, and an incompatible Codex resume or live host switch fails until a separately reviewed exact-replay export or explicitly lossy transition can replace backend-owned input providers and record the epoch, semantic boundary, and cache or context loss.

## Rationale

No implicit target keeps missing setup visible instead of silently spending work in Local Codex. Persisting only the first successfully verified choice gives subsequent startup a predictable default while later `connect` operations cannot unexpectedly replace it. Requiring replay evidence on both sides prevents target capability from inventing a prefix that a backend-managed source never exported.
