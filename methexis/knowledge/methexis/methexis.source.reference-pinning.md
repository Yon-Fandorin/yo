---
schema: methexis.knowledge/v1alpha1
id: methexis.source.reference-pinning
kind: rule
owner: methexis
sources:
  - id: methexis.source-model.reference-pinning
    revision: sha256:992aa0e974653d841f19f200ce1576d4f99005631922dc4bff2afc78eff38892
relations:
  depends_on:
    - methexis.knowledge.record-format
    - methexis.source.revision-identity
  validated_by:
    - tools/methexis/src/source/tests.rs::a_working_decision_change_can_only_demote_trusted_knowledge
    - tools/methexis/src/source/tests.rs::missing_and_mismatched_trusted_sources_are_distinct_failures
  applies_to:
    - tools/methexis/src/model.rs::SourceRef
    - tools/methexis/src/source/freshness.rs
---
# Exact Source reference pinning

## Statement

A KnowledgeUnit MUST pin each Source as the exact typed pair
`{SourceId, SourceRevision}`. A Source record owns its location, content or
external reference, and revision once; consumers MUST NOT copy that provenance
into each KnowledgeUnit.

A Source change MUST NOT advance a KnowledgeUnit implicitly. An author MUST
explicitly select the new SourceRevision and update the pin, producing a new
Knowledge RevisionId. That new revision MUST receive review, exact-revision
approval, and Checkpoint activation before it becomes trusted authority.
