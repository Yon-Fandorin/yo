---
schema: methexis.knowledge/v1alpha1
id: methexis.knowledge.unit
kind: definition
owner: methexis
sources:
  - id: methexis.knowledge-model.unit
    revision: sha256:718632476e920237a2a60b53dc720d74afa95946a726bdf9d206b5424ea2674a
relations:
  applies_to:
    - tools/methexis/src/model.rs::KnowledgeUnit
---
# Knowledge unit

## Statement

A KnowledgeUnit, commonly called a KU, is one independently changeable,
approvable, and invalidatable behavioral contract. It is neither an individual
sentence nor a whole design document.
