---
schema: methexis.knowledge/v1alpha1
id: methexis.knowledge.split-merge-supersession
kind: procedure
owner: methexis
sources:
  - id: methexis.knowledge-model.split-merge-supersession
    revision: sha256:6cb194ccd70fe22ece9344f1ddb9edceabb1e5dabc492526f747f0057cd4e8cf
relations:
  depends_on:
    - methexis.knowledge.semantic-continuity
    - methexis.relation.required-graph
  validated_by:
    - tools/methexis/tests/checkpoint_flow/failures.rs::checkpoint_cannot_select_a_replacement_with_its_superseded_unit
  applies_to:
    - tools/methexis/src/check.rs::validate_global
    - tools/methexis/src/checkpoint/validation.rs::select_from_foundation
---
# Split and merge supersession

## Statement

A split MUST create multiple new `KnowledgeId`s that each supersede the old
unit. A merge MUST create one new `KnowledgeId` that supersedes every merged
old unit. The transition MUST leave no required inbound relation unresolved,
MUST NOT select an old unit together with its replacement, and MUST receive a
human semantic-continuity review.

## Steps

1. Create the replacement IDs and record the required `supersedes` edges.
2. Resolve every required inbound relation whose target leaves the active
   selection, either by retargeting that relation or by removing or replacing
   its source unit in the same transition.
3. Validate target existence, required-graph and supersession acyclicity, and
   exclusion of old/replacement co-selection.
4. Obtain human review of the semantic mapping and activate the complete
   replacement selection in one Checkpoint transition.

## Completion Criteria

The split or merge is complete only when every replacement has its intended
stable identity, every required inbound obligation remains resolved, all
structural guards pass, human semantic continuity is accepted, and one active
Checkpoint selects the replacement set without any superseded unit.
