---
schema: methexis.knowledge/v1alpha1
id: methexis.migration.scope-preservation
kind: rule
owner: methexis
sources:
  - id: methexis.migration-model.scope-preservation
    revision: sha256:65d067b4c6893827f9cbc0fd1991e0c82115effb294d9bc01bd49023fc5f916e
relations:
  depends_on:
    - librarian.catalog.snapshot-ranking
    - librarian.delivery.storage-graduation
    - librarian.discovery.advisory-boundary
    - methexis.approval.current-record
    - methexis.approval.exact-revision-binding
    - methexis.architecture.process-boundaries
    - methexis.checkpoint.activation-transition
    - methexis.checkpoint.immutable-publication
    - methexis.checkpoint.local-cache
    - methexis.checkpoint.selection
    - methexis.context.build-identity
    - methexis.context.build-publication
    - methexis.context.build-reuse
    - methexis.context.bundle-packing
    - methexis.context.candidate-input
    - methexis.context.eligibility-failure
    - methexis.context.payload-manifest
    - methexis.context.request-selection
    - methexis.context.source-freshness
    - methexis.evaluation.pilot-corpus
    - methexis.graduation.repository-boundary
    - methexis.interface.agent-first
    - methexis.interface.atomic-publication
    - methexis.interface.operation-chain
    - methexis.knowledge.body-contract
    - methexis.knowledge.identity
    - methexis.knowledge.kind-extension
    - methexis.knowledge.kind-vocabulary
    - methexis.knowledge.record-format
    - methexis.knowledge.revision-identity
    - methexis.knowledge.semantic-continuity
    - methexis.knowledge.split-merge-supersession
    - methexis.knowledge.unit
    - methexis.knowledge.unit-boundary
    - methexis.migration.reversal-transition
    - methexis.pilot.deferred-scope
    - methexis.pilot.delivery-sequence
    - methexis.pilot.product-boundary
    - methexis.product.identity-terms
    - methexis.projection.korean-review
    - methexis.relation.required-graph
    - methexis.relation.vocabulary
    - methexis.source.kind-vocabulary
    - methexis.source.record-format
    - methexis.source.reference-pinning
    - methexis.source.revision-identity
    - methexis.status.approval
    - methexis.status.demotion-evidence
    - methexis.status.eligibility
    - methexis.status.negative-record
    - methexis.validation.bounded-success-output
    - methexis.validation.check-classes
    - methexis.validation.executable-evidence
    - methexis.validation.prospective-activation
    - methexis.validation.snapshot-construction
    - methexis.validation.tracked-artifacts
    - methexis.validation.working-tree-authority
    - methexis.workflow.self-hosting-boundary
---
# Post-migration SOT scope preservation

## Statement

The complete post-migration SOT owner set is encoded by this unit's typed `depends_on` relations. The required owners are:

- `librarian.catalog.snapshot-ranking`
- `librarian.delivery.storage-graduation`
- `librarian.discovery.advisory-boundary`
- `methexis.approval.current-record`
- `methexis.approval.exact-revision-binding`
- `methexis.architecture.process-boundaries`
- `methexis.checkpoint.activation-transition`
- `methexis.checkpoint.immutable-publication`
- `methexis.checkpoint.local-cache`
- `methexis.checkpoint.selection`
- `methexis.context.build-identity`
- `methexis.context.build-publication`
- `methexis.context.build-reuse`
- `methexis.context.bundle-packing`
- `methexis.context.candidate-input`
- `methexis.context.eligibility-failure`
- `methexis.context.payload-manifest`
- `methexis.context.request-selection`
- `methexis.context.source-freshness`
- `methexis.evaluation.pilot-corpus`
- `methexis.graduation.repository-boundary`
- `methexis.interface.agent-first`
- `methexis.interface.atomic-publication`
- `methexis.interface.operation-chain`
- `methexis.knowledge.body-contract`
- `methexis.knowledge.identity`
- `methexis.knowledge.kind-extension`
- `methexis.knowledge.kind-vocabulary`
- `methexis.knowledge.record-format`
- `methexis.knowledge.revision-identity`
- `methexis.knowledge.semantic-continuity`
- `methexis.knowledge.split-merge-supersession`
- `methexis.knowledge.unit`
- `methexis.knowledge.unit-boundary`
- `methexis.migration.reversal-transition`
- `methexis.pilot.deferred-scope`
- `methexis.pilot.delivery-sequence`
- `methexis.pilot.product-boundary`
- `methexis.product.identity-terms`
- `methexis.projection.korean-review`
- `methexis.relation.required-graph`
- `methexis.relation.vocabulary`
- `methexis.source.kind-vocabulary`
- `methexis.source.record-format`
- `methexis.source.reference-pinning`
- `methexis.source.revision-identity`
- `methexis.status.approval`
- `methexis.status.demotion-evidence`
- `methexis.status.eligibility`
- `methexis.status.negative-record`
- `methexis.validation.bounded-success-output`
- `methexis.validation.check-classes`
- `methexis.validation.executable-evidence`
- `methexis.validation.prospective-activation`
- `methexis.validation.snapshot-construction`
- `methexis.validation.tracked-artifacts`
- `methexis.validation.working-tree-authority`
- `methexis.workflow.self-hosting-boundary`

Every later Checkpoint transition MUST preserve this scope set by selecting an exact approved revision of `methexis.migration.scope-preservation` and its complete required closure, or an exact approved semantic successor whose reviewed relations assign every scope to an explicit owner. Omitting this unit, omitting any required owner, or selecting a pre-migration Checkpoint MUST NOT restore authority to historical prose in `docs-internal/design/sot-pilot.md`. A change of owner requires a reviewed revision of this unit or an explicit semantic successor and a forward compare-and-swap activation.
