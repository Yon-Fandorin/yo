---
schema: methexis.knowledge/v1alpha1
id: methexis.relation.required-graph
kind: rule
owner: methexis
sources:
  - id: methexis.relation-model.required-graph
    revision: sha256:1c3617b571053c997eaf3048cc38019b586a8ba2e2d28c12cef2641c56e576fd
relations:
  depends_on:
    - methexis.relation.vocabulary
  validated_by:
    - tools/methexis/tests/check.rs::global_failures_include_missing_targets_and_cycles
  applies_to:
    - tools/methexis/src/check.rs::validate_global
    - tools/methexis/src/check.rs::validate_cycles
---
# Required relation graph

## Statement

Authors MUST record only forward relations, and consumers MUST derive reverse
indexes. `depends_on` and `constrained_by` together form one required knowledge
graph, which MUST be acyclic. `supersedes` forms a separate graph and MUST also
be acyclic. `validated_by` and `applies_to` anchors MUST NOT participate in the
required knowledge graph.
