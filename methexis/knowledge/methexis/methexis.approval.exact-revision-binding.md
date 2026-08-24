---
schema: methexis.knowledge/v1alpha1
id: methexis.approval.exact-revision-binding
kind: rule
owner: methexis
sources:
  - id: methexis.approval-model.exact-revision-binding
    revision: sha256:525f1a9e88ac01390d1599f5890b889f99e9a9c721101744fdc8a1b06ce2afe8
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

Approval MUST bind one exact `RevisionId`, the reviewer `OwnerId`, review time, and one explicit review basis. Approval MUST NOT apply to a mutable `KnowledgeId` in general.

When the complete `canonical-approval-on-demand-projection/v1` capability is available, the `canonical` basis binds the exact canonical English Knowledge revision directly and MUST NOT require a Korean Projection. The `projection` basis additionally binds the exact Projection profile, compiler identity, and content hash. No operation may infer, silently change, or fall back between review bases.

Existing `methexis.approval/v1alpha1` Projection-backed records remain valid for the exact revisions and evidence they bind and require no bulk migration.
