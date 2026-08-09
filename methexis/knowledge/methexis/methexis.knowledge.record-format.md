---
schema: methexis.knowledge/v1alpha1
id: methexis.knowledge.record-format
kind: rule
owner: methexis
sources:
  - id: methexis.knowledge-model.record-format
    revision: sha256:e9259e2eb1b1f5bd2da3dc44bf32bcd0a3e79a15116ff91d965baf780e1fd1ef
relations:
  depends_on:
    - methexis.knowledge.identity
    - methexis.knowledge.unit
  validated_by:
    - tools/methexis/tests/check.rs::local_failures_are_aggregated_and_block_global_validation
    - tools/methexis/src/check.rs::tests::norway_rejects_yaml_merge_keys_at_the_typed_boundary
  applies_to:
    - tools/methexis/src/check.rs::load_records
    - tools/methexis/src/model.rs::KnowledgeMetadata
---
# Knowledge record format

## Statement

The Pilot MUST store one KU per Markdown file. Each file MUST contain closed,
typed YAML frontmatter that is validated as machine metadata and a constrained
canonical English Markdown body for meaning. Frontmatter MUST contain only the
schema, `KnowledgeId`, kind, `OwnerId`, exact Source references, and typed
relations. It MUST NOT duplicate a canonical statement from the body.

Canonical frontmatter MUST NOT use YAML merge keys. The loader MUST read the
`KnowledgeId` and `OwnerId` from record content rather than deriving either
identity from the physical location.
