---
schema: methexis.knowledge/v1alpha1
id: methexis.projection.korean-review
kind: rule
owner: methexis
sources:
  - id: methexis.projection-model.korean-review
    revision: sha256:25844581c9b9d743ee7ba7b88884dc29135842ae0f8d08ec9c5393c9359b343d
relations:
  depends_on:
    - methexis.knowledge.revision-identity
  validated_by:
    - tools/methexis/tests/review_flow/failures.rs::edited_projection_and_damaged_approval_are_structural_failures
  applies_to:
    - tools/methexis/src/review/records.rs::parse_projection
---
# Korean review Projection

## Statement

The Pilot MUST keep one generated Korean review Projection per `KnowledgeId`
under `methexis/review-projections/`. The Projection MUST bind the exact
`RevisionId`, Projection profile, compiler identity, deterministic request
lineage, and exact reviewed file bytes.

Direct edits, revision drift, or lineage drift MUST be structural failures.
The file MUST be regenerated from an explicit request rather than repaired by
an implicit or direct edit.
