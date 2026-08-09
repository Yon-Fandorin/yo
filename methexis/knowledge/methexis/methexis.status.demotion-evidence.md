---
schema: methexis.knowledge/v1alpha1
id: methexis.status.demotion-evidence
kind: rule
owner: methexis
sources:
  - id: methexis.status-model.demotion-evidence
    revision: sha256:0e99fa79cd8915914eec5dbafa494794cb972acb7c769d40b7f746f3e730427d
relations:
  depends_on:
    - methexis.relation.required-graph
    - methexis.source.reference-pinning
    - methexis.status.negative-record
  validated_by:
    - tools/methexis/src/source/tests.rs::a_working_decision_change_can_only_demote_trusted_knowledge
    - tools/methexis/src/source/tests.rs::missing_and_mismatched_trusted_sources_are_distinct_failures
    - tools/methexis/src/source/tests.rs::stale_required_source_propagates_only_to_dependents
  applies_to:
    - tools/methexis/src/source/freshness.rs::evaluate
    - tools/methexis/src/source/freshness.rs::propagate_required_dependents
---
# Status demotion guard and evidence

## Statement

The pre-transition and runtime status guard MUST map deterministic schema,
graph, integrity, or Checkpoint failures and explicit human invalidations to
`invalid`; unresolved review holds to `suspect`; and a pinned Source, evidence
result, retrieval, or attestation that no longer satisfies the freshness input
approved for the Knowledge revision to `stale`. Its ineligible winning
condition order MUST be `invalid > suspect > stale`. Durable negative inputs
are owned by `methexis.status.negative-record`.

A current working-tree or host observation MAY only demote the guard outcome;
it MUST NOT grant approval or activation. Every guard outcome MUST include
machine-readable evidence for its winning condition so that precedence and
transitions are testable.

A resolution started after a pinned Source change MUST block the affected
knowledge and Projections and MUST mark every affected Checkpoint
degraded. The winning ineligible state MUST propagate through the selected
required graph only toward selected dependents that transitively require the
affected unit. Unaffected prerequisites, siblings, and unrelated approved
knowledge MUST remain eligible. A Source change concurrent with resolution
MUST follow the immutable snapshot and final-revalidation rules owned by
`SOT-007`, rather than being silently accepted or retried against mixed
observations. This unit routes that concurrent-change case to `SOT-007`; it does
not copy or take ownership of those rules.
