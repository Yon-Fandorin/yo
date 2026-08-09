---
schema: methexis.knowledge/v1alpha1
id: methexis.validation.working-tree-authority
kind: rule
owner: methexis
sources:
  - id: methexis.validation-model.working-tree-authority
    revision: sha256:c48436aa9345ad2ce53e44e930e7b7f8665b86aec11eb37686f5ff5781d591e1
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
