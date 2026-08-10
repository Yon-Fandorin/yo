---
schema: methexis.knowledge/v1alpha1
id: methexis.migration.complete-transition
kind: rule
owner: methexis
sources:
  - id: methexis.migration-model.complete-transition
    revision: sha256:30c09256e2eca919885aa95769faeff492b81abb9dc5c6a623a9c6cd0a9898c4
relations:
  depends_on:
    - methexis.migration.scope-preservation
---
# Complete SOT authority transition

## Statement

Before the complete migration becomes trusted, `docs-internal/design/sot-pilot.md` remains the sole authority for every scope not already delegated to an active semantic KnowledgeUnit. Existing active KnowledgeUnit revisions remain authoritative for their already delegated scopes.

The complete migration MUST be one forward compare-and-swap Checkpoint transition that selects an exact approved revision of `methexis.migration.complete-transition` and its complete required closure. That closure MUST include `methexis.migration.scope-preservation`, `methexis.migration.reversal-transition`, and every exact scope owner required by `methexis.migration.scope-preservation`. Partial selection transfers no remaining document-owned scope.

Once that exact transition becomes trusted, the scope-owner KnowledgeUnits become the sole authority for their assigned scopes and `docs-internal/design/sot-pilot.md` becomes a non-authoritative routing Projection. The currently authoritative revisions remain authoritative until the exact replacement transition becomes trusted; replacement revisions need not already be active.
