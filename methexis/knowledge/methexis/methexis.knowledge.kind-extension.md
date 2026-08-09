---
schema: methexis.knowledge/v1alpha1
id: methexis.knowledge.kind-extension
kind: rule
owner: methexis
sources:
  - id: methexis.knowledge-model.kind-extension
    revision: sha256:f6690351aa3fee9e8d29b16bad8d372755ad1e2bf754ccc6be5c4651a72cf101
relations:
  depends_on:
    - methexis.knowledge.kind-vocabulary
  applies_to:
    - tools/methexis/src/model.rs::KnowledgeKind
---
# Knowledge kind extension

## Statement

Knowledge records MUST use only the closed kind vocabulary. Classification
friction MUST be recorded before a new kind is admitted. Catch-all kinds such
as `misc` MUST NOT be admitted.
