---
schema: methexis.knowledge/v1alpha1
id: methexis.migration.complete-transition
kind: rule
owner: methexis
sources:
  - id: methexis.migration-model.complete-transition
    revision: sha256:0bf8a925ffde811a3f290310b71b96ad5e7637c26f5de246031fb4b4c8045708
relations:
  depends_on:
    - methexis.migration.scope-preservation
---
# SOT authority closure

## Statement

All accepted SOT scope is authoritative only through exact approved KnowledgeUnit revisions selected by the tracked active Checkpoint. `methexis.migration.complete-transition` remains the root that closes over `methexis.migration.scope-preservation`, `methexis.migration.reversal-transition`, and every exact owner required by the scope registry.

Any replacement MUST use one forward compare-and-swap Checkpoint transition. The currently selected revisions remain authoritative until the exact replacement is trusted; a partial, working-tree-only, or untrusted selection transfers no authority. Repository prose outside the active KnowledgeUnit closure is not a fallback authority.
