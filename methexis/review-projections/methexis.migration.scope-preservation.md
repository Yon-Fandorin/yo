---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.migration.scope-preservation
revision: sha256:a0bfb1328909bc4d466931507b2798dfb339ff4c30641ffc463da0a0c67f4dbc
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:f34ee588ee85b46ee17cd634123e2704a715de178473b17b5ba94e95e680f743
---
# Korean Review Projection

## Translation

이 KU의 depends_on closure가 migration 이후의 완전한 SOT owner 집합입니다. 이후 모든 Checkpoint는 이 KU와 전체 closure를 보존해야 하며, owner를 바꾸려면 모든 scope를 명시적으로 다시 배정한 reviewed revision 또는 successor와 forward CAS activation이 필요합니다. 누락이나 pre-migration Checkpoint가 옛 문서 prose의 권위를 되살리지 않습니다.

### 전체 정본 원문 대조

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
