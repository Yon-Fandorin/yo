---
schema: methexis.knowledge/v1alpha1
id: methexis.status.approval
kind: definition
owner: methexis
sources:
  - id: methexis.status-model.approval
    revision: sha256:6ba7f59c6b9617abad9b56382229c6e556e516dea2c2d65e5b79bc5ec215ce5e
relations:
  depends_on:
    - methexis.approval.current-record
    - methexis.approval.exact-revision-binding
    - methexis.knowledge.revision-identity
  validated_by:
    - tools/methexis/tests/review_flow/contract.rs::projection_review_and_approval_form_an_idempotent_proposal_flow
    - tools/methexis/tests/checkpoint_flow/contract.rs::trusted_activation_becomes_active_when_decision_sources_are_fresh
  applies_to:
    - tools/methexis/src/check/runner.rs::check_repository_selected
---
# Derived approval status

## Statement

Approval status MUST be a derived axis separate from context eligibility. Its
closed labels are `draft` and `approved`. Consumers MUST evaluate this axis
independently from eligibility, and `approved` alone MUST NOT imply that a
revision is eligible for normal context.

`methexis.approval.exact-revision-binding` and
`methexis.approval.current-record` remain the sole owners of the conditions for
deriving these labels. This status definition MUST route to those contracts and
MUST NOT redefine, duplicate, or weaken their exact-revision, proposal, or
trusted-integration boundaries.
