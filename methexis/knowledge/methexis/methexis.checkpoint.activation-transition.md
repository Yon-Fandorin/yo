---
schema: methexis.knowledge/v1alpha1
id: methexis.checkpoint.activation-transition
kind: procedure
owner: methexis
sources:
  - id: methexis.checkpoint-model.activation-transition
    revision: sha256:7fc71b9ae1bec5d4f654e40681dac9c56f496dd5b4ca74cd33a4f44d9480de98
relations:
  depends_on:
    - methexis.checkpoint.immutable-publication
    - methexis.status.demotion-evidence
  constrained_by:
    - methexis.output.checkpoint-delta-summary
  validated_by:
    - tools/methexis/tests/checkpoint_flow/replacement.rs::activation_replacement_requires_the_exact_active_record_hash
    - tools/methexis/tests/checkpoint_flow/contract.rs::trusted_activation_becomes_active_when_decision_sources_are_fresh
    - tools/methexis/tests/checkpoint_flow/lineage.rs::self_consistent_but_unreproducible_checkpoint_is_rejected
    - tools/methexis/tests/checkpoint_flow/prospective.rs::staged_replacement_rejects_degraded_source_freshness
  applies_to:
    - tools/methexis/src/checkpoint/operations.rs::propose_activation
    - tools/methexis/src/checkpoint/records.rs::build_active
    - tools/methexis/src/checkpoint/evaluation.rs::evaluate
---
# Checkpoint activation transition

## Statement

A tracked active-Checkpoint record MUST point to one exact `CheckpointId` and its content hash. The Checkpoint, rather than the active record, owns its trusted authority-basis commit and reproducible selection lineage. A replacement active record MUST additionally bind the exact prior trusted active-record content hash as its compare-and-swap predecessor. Initial activation MUST have no predecessor, and neither transition has a force path.

Activation MUST remain a separate, reviewed Git change that adds or updates the immutable Checkpoint and active record together. A proposal is not authority; the transition becomes authoritative only when the accepted commit is reachable from the configured trusted integration ref.

At the pre-integration gate, activation MUST invoke and obey the pre-transition demotion guard owned by `methexis.status.demotion-evidence` for the complete selected closure. Its required dependency closure supplies durable negative inputs through `methexis.status.negative-record`. An `invalid`, `suspect`, or `stale` result MUST block the prospective transition. The guard can only demote and MUST NOT grant approval or activation. This transition MUST NOT depend on or invoke the post-transition `methexis.status.eligibility` contract: final `active` or `inactive` membership is derived from a trusted active Checkpoint only after this transition.

## Steps

1. Verify that the request identifies the exact immutable Checkpoint and that its authority-basis commit equals the current pinned trusted commit.
2. Reproduce the Checkpoint from its recorded commit and reject any lineage, byte, ID, hash, approval-closure, or predecessor mismatch.
3. Build the canonical active record with its exact Checkpoint link and compare-and-swap predecessor, and publish only the reviewable proposal.
4. Before integration, invoke `methexis.status.demotion-evidence` for the complete selected closure and reject every `invalid`, `suspect`, or `stale` result using its winning-condition evidence.
5. Review and integrate the exact Checkpoint and active-record transition through the repository workflow.
6. From trusted integration, require the authority-basis commit to remain readable and ancestral, reproduce the Checkpoint again, and verify that the current approved required closure and pre-transition demotion guard still pass before deriving the Checkpoint active. Final per-Knowledge eligibility remains owned by `methexis.status.eligibility`.

## Completion Criteria

The transition is complete only when the trusted active record names the exact immutable Checkpoint, every lineage and compare-and-swap check passes, the accepted commit is reachable from trusted integration, the current approved required closure reproduces the Checkpoint, the pre-transition `methexis.status.demotion-evidence` guard passes, and no fallback, force replacement, partial publication, proposal-only state, or post-transition eligibility dependency is treated as transition authority.
