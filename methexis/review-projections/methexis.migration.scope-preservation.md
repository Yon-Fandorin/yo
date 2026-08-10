---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.migration.scope-preservation
revision: sha256:3936ca89e5cd006aadecedd66095385341dd61d3fa066ba02fa5cd033c850204
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:b95ec9a654ef2451f78d75095e98152a736f5847f6a0085042643eb7cf957eca
---
# Korean Review Projection

## Translation

이 unit의 typed depends_on relation이 현재 SOT owner 전체를 정의합니다. 필수 owner는 다음과 같습니다.

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

이후 Checkpoint도 이 unit의 정확히 승인된 revision과 전체 required closure, 또는 모든 scope에 명시적 owner를 배정한 reviewed successor를 선택해야 합니다. owner 변경은 이 unit의 reviewed revision 또는 명시적 semantic successor와 forward compare-and-swap activation을 모두 거쳐야 합니다. owner가 빠지면 authority validation은 실패하며, active KU closure 밖의 prose는 누락 owner를 대신할 수 없습니다.
