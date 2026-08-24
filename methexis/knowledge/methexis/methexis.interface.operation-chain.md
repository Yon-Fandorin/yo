---
schema: methexis.knowledge/v1alpha1
id: methexis.interface.operation-chain
kind: rule
owner: methexis
sources:
  - id: methexis.interface-model.operation-chain
    revision: sha256:1c514912bbeb078f89943664d5441767afd18132af17e8a7fc103bf117b9588d
---
# Methexis operation chain and authority boundaries

## Statement

`canonical-approval-on-demand-projection/v1` is exposed only for the complete minimal flow: `author-revision` writes Source and canonical English Knowledge Drafts; after repository-owned semantic review clears, `prepare-approval` may bind the exact canonical revision directly without a Projection or review packet; and `approve` retains the separate exact-human-authorization boundary.

When a human explicitly requests additional Korean understanding, `project-review` and `build-review` form an optional branch that creates or reuses the exact Korean pair before `prepare-approval` emits a Projection-basis request. No operation may generate a Projection implicitly, switch review basis, or treat review as approval.

The capability selects the current operation path and creates no durable authority or artifact lineage. Existing `semantic-first-ko-on-demand/v1` and `methexis.approval/v1alpha1` Projection-backed records remain compatible for their exact revisions without bulk migration.

Agent-review procedure, reviewer-session handling, and review evidence are owned exclusively by the repository workflow authority. Methexis consumes only the resulting workflow disposition and does not define a second provider-attestation or reviewer-routing policy. Other prepare, Checkpoint, activation, validation, and ContextBuild boundaries are unchanged except that each consumes the approval record's explicit review basis.
