---
schema: methexis.knowledge/v1alpha1
id: methexis.checkpoint.immutable-publication
kind: rule
owner: methexis
sources:
  - id: methexis.checkpoint-model.immutable-publication
    revision: sha256:279f84c81eef5c0adb74d5358a25150ddc61b851c3839d745fd6c26535dae366
relations:
  depends_on:
    - methexis.checkpoint.selection
  constrained_by:
    - methexis.output.checkpoint-delta-summary
  validated_by:
    - tools/methexis/tests/checkpoint_flow/lineage.rs::self_consistent_but_unreproducible_checkpoint_is_rejected
    - tools/methexis/tests/checkpoint_flow/failures.rs::damaged_checkpoint_is_rejected_without_active_output
  applies_to:
    - tools/methexis/src/checkpoint/operations.rs::create
    - tools/methexis/src/checkpoint/records.rs::build_checkpoint
    - tools/methexis/src/checkpoint/storage.rs::publish_immutable
---
# Immutable Checkpoint publication

## Statement

Checkpoint creation MUST resolve one configured trusted Git ref to an exact
commit, read the required Source, Knowledge, Projection, and approval blobs from
that pinned snapshot without checking it out, and select the approved closure
from those captured bytes only.

The proposal MUST use a deterministic canonical record that binds its schema,
trusted commit, historical Source-status marker, roots, selected exact
revisions, and selection reasons into its `CheckpointId`. Before publication,
the same record MUST be reproducible from the recorded commit.

The proposal MUST be published as an immutable create-if-absent artifact. An
existing identical artifact MAY be reused. Different existing bytes, invalid
closure, or unreadable input MUST fail without replacement, fallback, or a
partial alternative Checkpoint.
