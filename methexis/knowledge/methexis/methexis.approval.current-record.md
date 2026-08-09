---
schema: methexis.knowledge/v1alpha1
id: methexis.approval.current-record
kind: rule
owner: methexis
sources:
  - id: methexis.approval-model.current-record
    revision: sha256:8c1e070ac82cbaba653bc3ea321df47a3ec1d0e4020fd48dee374ea44f1b2d3b
relations:
  depends_on:
    - methexis.approval.exact-revision-binding
  validated_by:
    - tools/methexis/tests/review_flow/contract.rs::projection_review_and_approval_form_an_idempotent_proposal_flow
    - tools/methexis/tests/review_flow/failures.rs::approval_rejects_wrong_evidence_reviewer_and_time
  applies_to:
    - tools/methexis/src/review/operations.rs::record_approval
    - tools/methexis/src/review/validation.rs::validate_records
---
# Current approval record

## Statement

Each `KnowledgeId` MUST have at most one current approval record under
`methexis/approvals/`. When the current revision differs from that record, the
unit MUST be Draft. Git history retains prior approval records; the Pilot MUST
NOT create an unbounded file per historical revision.

Writing identical bytes MUST be idempotent. Replacing different bytes MUST
require the exact prior `RevisionId` as a compare-and-swap precondition, and
there MUST be no force path. A matching record in a working tree or proposed
branch is only approval evidence for review; it MUST NOT produce effective
approved state until loaded from the configured trusted integration commit.
