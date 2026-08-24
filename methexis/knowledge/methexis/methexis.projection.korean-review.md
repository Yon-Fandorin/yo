---
schema: methexis.knowledge/v1alpha1
id: methexis.projection.korean-review
kind: rule
owner: methexis
sources:
  - id: methexis.projection-model.korean-review
    revision: sha256:434651c2f922ce8133ce5f589234d70d1b49ca14a82d3756b1e716aaef2ddcbf
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

Source records and canonical English Knowledge are the semantic authoring and agent-review surface. The complete `canonical-approval-on-demand-projection/v1` capability makes the Korean review Projection an optional human-understanding aid rather than a prerequisite for every approval. Without that capability, the existing `semantic-first-ko-on-demand/v1` flow remains controlling.

Under the new capability, no authoring, approval, activation, validation, or ContextBuild operation may create a Korean Projection implicitly. When a human explicitly requests additional Korean understanding, `project-review` reuses a fresh exact-revision Projection when present or creates/replaces one when absent or stale. The request names the exact current `RevisionId` and predecessor hash when replacing. The Projection binds that revision, profile, compiler, deterministic request lineage, and exact bytes. Direct edits, malformed records, or lineage drift fail closed.

A canonical-basis approval requires no Projection. An unreferenced stale Projection is ineligible as review evidence but MUST NOT block a matching canonical approval or activation. A Projection-basis approval still requires the exact English-plus-Korean pair and binds the Projection hash; semantic change returns to English review and translation-only change repeats human review. Existing legacy artifacts remain valid for their exact approved revisions without bulk migration.
