---
schema: methexis.knowledge/v1alpha1
id: methexis.checkpoint.activation-transition
kind: procedure
owner: methexis
sources:
  - id: methexis.checkpoint-model.activation-transition
    revision: sha256:6651c9634b9c0bb6148b9267189b407fc04f8f0f4daf4f9289e7e0854f3f9fa4
relations:
  depends_on:
    - methexis.checkpoint.immutable-publication
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

A tracked active-Checkpoint record MUST point to one exact `CheckpointId` and
its content hash. The Checkpoint, rather than the active record, owns its trusted
authority-basis commit and reproducible selection lineage. A replacement active
record MUST additionally bind the exact prior trusted active-record content
hash as its compare-and-swap predecessor. Initial activation MUST have no
predecessor, and neither transition has a force path.

Activation MUST remain a separate, reviewed Git change that adds or updates the
immutable Checkpoint and active record together. A proposal is not authority;
the transition becomes authoritative only when the accepted commit is reachable
from the configured trusted integration ref.

At the pre-integration gate, activation MUST invoke and obey the current
Source-freshness guard owned by `SOT-006`. The guard's inputs, eligibility
states, precedence, demotion evidence, context exclusion, and failure semantics
remain solely owned by `SOT-006`; this procedure does not redefine them. A
non-passing result MUST block the prospective transition. After integration,
the trusted Checkpoint MUST be derived `active` only when that externally owned
guard passes.

## Steps

1. Verify that the request identifies the exact immutable Checkpoint and that
   its authority-basis commit equals the current pinned trusted commit.
2. Reproduce the Checkpoint from its recorded commit and reject any lineage,
   byte, ID, hash, approval-closure, or predecessor mismatch.
3. Build the canonical active record with its exact Checkpoint link and
   compare-and-swap predecessor, and publish only the reviewable proposal.
4. Before integration, invoke the current `SOT-006` Source-freshness guard for
   the complete selected closure and reject every non-passing result according
   to that guard's authority.
5. Review and integrate the exact Checkpoint and active-record transition
   through the repository workflow.
6. From trusted integration, require the authority-basis commit to remain
   readable and ancestral, reproduce the Checkpoint again, and verify that the
   current approved required closure and Source freshness still match before
   deriving it active.

## Completion Criteria

The transition is complete only when the trusted active record names the exact
immutable Checkpoint, every lineage and compare-and-swap check passes, the
accepted commit is reachable from trusted integration, the current approved
closure reproduces the Checkpoint, the required `SOT-006` Source-freshness
guard passes, and no fallback, force replacement, partial publication, or
proposal-only state is treated as authority.
