---
schema: methexis.knowledge/v1alpha1
id: methexis.knowledge.identity
kind: rule
owner: methexis
sources:
  - id: methexis.knowledge-model.identity
    revision: sha256:24200a9ccc29a7829af72325be15ebec2f5aa87ab4cff49c3984e2978d4551a5
relations:
  depends_on:
    - methexis.knowledge.unit
  validated_by:
    - tools/methexis/tests/check.rs::local_failures_are_aggregated_and_block_global_validation
    - tools/methexis/tests/check.rs::repeated_checks_and_physical_relocation_preserve_identity
  applies_to:
    - tools/methexis/src/check.rs::is_semantic_id
---
# Knowledge identity

## Statement

Every KU MUST have a stable semantic `KnowledgeId` read from record content.
Its directory and filename are mutable organizational hints and MUST NOT define
identity. Moving a valid record MUST preserve its `KnowledgeId`.

A `KnowledgeId` MUST contain lowercase dot-separated semantic segments. Each
segment MUST start with an ASCII letter, end with an ASCII letter or digit, and
contain only lowercase ASCII letters, digits, or single internal hyphens. An ID
MUST NOT encode the physical path, record kind, revision, or first consumer.
