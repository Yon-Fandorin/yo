---
schema: methexis.knowledge/v1alpha1
id: methexis.approval.exact-revision-binding
kind: rule
owner: methexis
sources:
  - id: methexis.approval-model.exact-revision-binding
    revision: sha256:033bb7cd862d519529d40dcd389c434e4ba01512a27cfa1e1b8e3659defac0de
relations:
  depends_on:
    - methexis.knowledge.revision-identity
    - methexis.projection.korean-review
  validated_by:
    - tools/methexis/tests/review_flow/contract.rs::projection_review_and_approval_form_an_idempotent_proposal_flow
  applies_to:
    - tools/methexis/src/review/validation.rs::validate_records
---
# Exact-revision approval binding

## Statement

An approval MUST bind one exact `RevisionId`, the reviewer `OwnerId`, review
time, and the profile, compiler identity, and hash of the Korean review
Projection. Approval MUST NOT apply to a mutable `KnowledgeId` in general.
