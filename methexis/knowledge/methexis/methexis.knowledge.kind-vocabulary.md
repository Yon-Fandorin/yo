---
schema: methexis.knowledge/v1alpha1
id: methexis.knowledge.kind-vocabulary
kind: definition
owner: methexis
sources:
  - id: methexis.knowledge-model.kind-vocabulary
    revision: sha256:323a85db0168e317a609a61dee61cd847bf9b25db5e5a11b49caf6ab43daabe5
relations:
  depends_on:
    - methexis.knowledge.unit
  applies_to:
    - tools/methexis/src/model.rs::KnowledgeKind
---
# Knowledge kind vocabulary

## Statement

The closed initial KU kinds are:

- `definition`: a shared term or meaning;
- `rule`: required behavior, a constraint, or an invariant;
- `decision`: a selected direction together with its rationale; and
- `procedure`: ordered work with a completion condition.
