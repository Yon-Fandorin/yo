---
schema: methexis.knowledge/v1alpha1
id: methexis.knowledge.semantic-continuity
kind: rule
owner: methexis
sources:
  - id: methexis.knowledge-model.semantic-continuity
    revision: sha256:c6f6c14f8ad5e2e7b136df4e9bac702085095d5b297c602fef53d2cb60ceec49
relations:
  depends_on:
    - methexis.knowledge.revision-identity
    - methexis.knowledge.unit-boundary
    - methexis.relation.required-graph
    - methexis.relation.vocabulary
  validated_by:
    - tools/methexis/tests/check.rs::global_failures_include_missing_targets_and_cycles
    - tools/methexis/tests/checkpoint_flow/failures.rs::checkpoint_cannot_select_a_replacement_with_its_superseded_unit
  applies_to:
    - tools/methexis/src/check.rs::validate_global
    - tools/methexis/src/checkpoint/validation.rs::select_from_foundation
---
# Knowledge semantic continuity

## Statement

A revision MUST remain under the same `KnowledgeId` only while it answers the
same semantic question and every existing inbound relation still identifies
the same obligation. Clarification, tighter wording, and changed outcomes for
that same obligation are new revisions of the existing ID.

A changed subject or obligation that would make an existing inbound relation
silently acquire different meaning MUST use a new `KnowledgeId` related by
`supersedes`. Every supersession target MUST exist, the supersession graph MUST
be acyclic, old and replacement units MUST NOT be active together, and no
removed ID MUST leave a required inbound relation unresolved.

Deterministic validation establishes only those structural guarantees.
Librarian MAY flag overlapping anchors or similar meaning only as a possible
unexplained replacement, and a human reviewer MUST own the semantic-continuity
decision.
