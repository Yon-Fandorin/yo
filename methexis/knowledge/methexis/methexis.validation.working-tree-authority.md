---
schema: methexis.knowledge/v1alpha1
id: methexis.validation.working-tree-authority
kind: rule
owner: methexis
sources:
  - id: methexis.validation-model.working-tree-authority
    revision: sha256:31d6d8c9b8ee46a0f0183229f3ce17c9c6c8a3071120bcca316059bbe36f9626
relations:
  depends_on:
    - methexis.status.approval
    - methexis.status.eligibility
    - methexis.validation.snapshot-construction
  validated_by:
    - tools/methexis/tests/review_flow/contract.rs::projection_review_and_approval_form_an_idempotent_proposal_flow
    - tools/methexis/tests/review_flow/failures.rs::invalid_requests_and_stale_projection_fail_without_partial_authority
    - tools/methexis/tests/checkpoint_flow/contract.rs::trusted_activation_becomes_active_when_decision_sources_are_fresh
    - tools/methexis/tests/checkpoint_flow/contract.rs::trusted_code_activation_degrades_without_losing_approval_on_byte_drift
  applies_to:
    - tools/methexis/src/review/validation.rs::validate_records
    - tools/methexis/src/check/runner.rs::check_repository_selected
---
# Working-tree validation is not authority

## Statement

After structural record validation owned by
`methexis.validation.snapshot-construction` succeeds, working-tree Fast Check
MUST evaluate Korean review Projections and approval proposals against the
current Draft Knowledge and typed Sources. It MAY report proposal evidence as
`matching_proposal`, `stale_proposal`, or missing, but local evidence MUST NOT
grant trusted approval or activation. This unit MUST NOT redefine structural
record validation.

Fast Check MUST consume the approval axis derived by
`methexis.status.approval` and final eligibility derived by
`methexis.status.eligibility`; it MUST NOT redefine either status. Current
working-tree or host observations MAY contribute only through the demotion
guard routed by those status contracts and MUST NOT promote Draft, inactive, or
unapproved content.

A successful report MUST identify its own authority as Draft even when it
includes statuses derived from trusted integration. If trusted status
evaluation fails, Fast Check MUST return failure and MUST NOT fall back to
local proposal evidence as trusted state.

Records reachable from the repository-local `refs/heads/develop` are the only
approval authority in the current Pilot. Task input, environment variables, and
the invoking agent MUST NOT override it. At the start of an operation, the ref
is resolved once to an exact commit; that pinned snapshot is the only authority
used for computation and its commit is recorded in every result. An operation
that promises final authority stability MAY reread only the configured ref and
active-record identities before returning. A mismatch fails the pinned
operation and never switches it to the newer snapshot. An internal injected
policy MAY be used by isolated tests but is not a production input surface.

Authority reads MUST use the system Git executable with caller Git
configuration and environment removed. Replacement refs and graft-like object
substitution MUST be disabled, so the recorded object ID and materialized tree
cannot diverge.

A Task commit, proposed Slice commit, working-tree state, or branch name
supplied by the caller is never authority. Supporting a human-approved Wave
commit as a temporary trust anchor is deferred until repository policy owns a
non-caller-controlled configuration surface.

Knowledge, Source, approval, Checkpoint, and active-Checkpoint records MUST be
tracked. Proposed branch and working-tree edits are Draft inputs until the
repository approval workflow integrates them into the configured trust anchor.
A database or local file MAY be a rebuildable index or cache, but MUST NOT
become a second writable authority. The compiler consumes a storage-neutral
immutable `KnowledgeSnapshot`.
