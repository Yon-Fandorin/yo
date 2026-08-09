---
schema: methexis.knowledge/v1alpha1
id: methexis.relation.vocabulary
kind: definition
owner: methexis
sources:
  - id: methexis.relation-model.vocabulary
    revision: sha256:405254fe19b92ccfbf1a0e1ab3059b7a677eb770eb5de5f65f50f16d48dd977b
relations:
  depends_on:
    - methexis.knowledge.unit
  validated_by:
    - tools/methexis/tests/check.rs::global_failures_include_missing_targets_and_cycles
  applies_to:
    - tools/methexis/src/model.rs::Relations
---
# Relation vocabulary

## Statement

The canonical relation vocabulary is closed to these five typed relations:

- `depends_on` targets a KnowledgeUnit required for completeness;
- `constrained_by` targets a KnowledgeUnit that limits allowed behavior;
- `validated_by` targets a test or fixture that supplies executable evidence;
- `applies_to` targets a code anchor in scope, such as a file, module, symbol,
  or mode;
- `supersedes` targets a KnowledgeUnit whose semantic identity is replaced.

Derivation and support belong to Source provenance. Translation and
summarization belong to Projection lineage. A weak `related_to` signal is
advisory Librarian discovery data, not a canonical relation, and MUST NOT
affect SOT eligibility or invalidation.
