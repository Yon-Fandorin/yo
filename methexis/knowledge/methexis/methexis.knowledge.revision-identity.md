---
schema: methexis.knowledge/v1alpha1
id: methexis.knowledge.revision-identity
kind: rule
owner: methexis
sources:
  - id: methexis.knowledge-model.revision-identity
    revision: sha256:4dc984e4bb685e7f6871d597f318e4c60581c98ab6d2fb451ffaeb98744dc9fe
relations:
  depends_on:
    - methexis.knowledge.record-format
    - methexis.relation.vocabulary
    - methexis.source.reference-pinning
  validated_by:
    - tools/methexis/src/check.rs::tests::semantic_revision_ignores_yaml_order_and_line_endings
    - tools/methexis/src/check.rs::tests::semantic_revision_sorts_sources_and_typed_relations
    - tools/methexis/src/check.rs::tests::body_change_changes_revision
    - tools/methexis/src/check.rs::tests::semantic_revision_has_a_golden_digest
  applies_to:
    - tools/methexis/src/check.rs::knowledge_revision
---
# Knowledge revision identity

## Statement

`RevisionId` identifies the exact canonical meaning of one stable KnowledgeId.
It MUST be encoded as `sha256:<lowercase-hex>` over one unambiguous,
length-delimited semantic representation containing schema version,
KnowledgeId, kind, owner, canonical body, sorted exact Source references, and
each closed relation type with its sorted target references, including an empty
list when that relation has no targets.

The loader MUST normalize CRLF and bare CR line endings to LF before hashing.
Physical path, YAML key order or formatting, generation time, and original
line-ending representation MUST NOT affect RevisionId. Every other canonical
body byte MUST remain meaningful.
