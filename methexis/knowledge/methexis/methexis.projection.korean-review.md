---
schema: methexis.knowledge/v1alpha1
id: methexis.projection.korean-review
kind: rule
owner: methexis
sources:
  - id: methexis.projection-model.korean-review
    revision: sha256:051baf7734b890170422928469ffe1d0f437a0567424e468dd3545762fedb7ee
relations:
  depends_on:
    - methexis.knowledge.revision-identity
  validated_by:
    - tools/methexis/tests/review_flow/failures.rs::edited_projection_and_damaged_approval_are_structural_failures
  applies_to:
    - tools/methexis/src/review/records.rs::parse_projection
---
# On-demand Korean review Projection

## Statement

Source records and canonical English Knowledge are the semantic authoring and agent-review surface. This flow is available only with the complete `semantic-first-ko-on-demand/v1` capability. The capability selects the current operation path; it does not create durable authority or artifact lineage. Without it, the legacy flow remains controlling and still requires exact human approval.

With the capability, `author-revision` changes Source and Knowledge only and does not accept, generate, copy forward, or replace Korean Markdown. An existing stale Projection is not current review or approval evidence and authority validation rejects it.

After the exact clean semantic candidate clears required review, `project-review` creates or replaces the one tracked Korean Projection only on explicit human request. The request names the exact current `RevisionId` and predecessor hash when replacing. The Projection binds that revision, profile, compiler, deterministic request lineage, and exact bytes. Direct edits, revision drift, or lineage drift are structural failures.

The human reviews the exact English revision and Korean Projection together, and approval binds the revision and Projection hash. Semantic change returns to English-only review; translation-only change repeats human review. Existing legacy artifacts remain valid for their exact approved revisions without bulk migration.
