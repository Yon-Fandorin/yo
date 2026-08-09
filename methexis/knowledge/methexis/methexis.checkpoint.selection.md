---
schema: methexis.knowledge/v1alpha1
id: methexis.checkpoint.selection
kind: rule
owner: methexis
sources:
  - id: methexis.checkpoint-model.selection
    revision: sha256:03e4440557a47513755d489e04953f27aad3bc10738371ff7aba51b8402e5b11
relations:
  depends_on:
    - methexis.approval.current-record
    - methexis.approval.exact-revision-binding
    - methexis.knowledge.semantic-continuity
    - methexis.relation.required-graph
    - methexis.relation.vocabulary
  validated_by:
    - tools/methexis/tests/checkpoint_flow/failures.rs::unapproved_root_and_moved_trust_anchor_fail_closed
    - tools/methexis/tests/checkpoint_flow/failures.rs::checkpoint_cannot_select_a_replacement_with_its_superseded_unit
  applies_to:
    - tools/methexis/src/checkpoint/validation.rs::select_from_foundation
    - tools/methexis/src/checkpoint/records.rs::build_checkpoint
---
# Checkpoint selection

## Statement

A `Checkpoint` MUST pin a consistent map from approved `KnowledgeId`s to their
exact `RevisionId`s. Its request MUST name at least one explicit root. Selection
MUST include every root and the complete transitive `depends_on` and
`constrained_by` closure; `validated_by` and `applies_to` MUST NOT select units.

Every selected revision MUST have exact trusted approval. A missing root,
missing required dependency, or unapproved member MUST fail the selection
without producing a partial Checkpoint. A replacement and a unit it
`supersedes` MUST NOT be selected together.

The Checkpoint MUST retain the historical `source_status: not_evaluated` input
marker. Source freshness and the resulting active or degraded state are current
derived guards and MUST NOT be authored into Checkpoint selection state.
