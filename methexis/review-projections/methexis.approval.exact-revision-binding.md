---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.approval.exact-revision-binding
revision: sha256:a67bffbf9c9e39451139289a0223407e733cdddc5949833d390b1e7f8156ad18
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:4eca575d3130efc9df9d9dd69cf5665f4b6a92866e062a907adf57a4d2b454aa
---
# Korean Review Projection

## Translation

# 정확한 리비전 승인 결속

## 명세

승인은 하나의 정확한 `RevisionId`, 검토자 `OwnerId`, 검토 시각과 명시적인 검토 근거 하나에 결속되어야 한다. 승인은 일반적인 가변 `KnowledgeId`에 적용되어서는 안 된다.

완전한 `canonical-approval-on-demand-projection/v1` capability를 사용할 수 있을 때, `canonical` 근거는 정확한 canonical 영문 Knowledge 리비전에 직접 결속되며 한국어 Projection을 요구해서는 안 된다. `projection` 근거는 여기에 더해 정확한 Projection profile, compiler identity, content hash에 결속된다. 어떤 operation도 검토 근거를 추론하거나, 조용히 바꾸거나, 다른 근거로 fallback해서는 안 된다.

기존 `methexis.approval/v1alpha1` Projection 기반 record는 자신이 결속한 정확한 리비전과 증거에 대해 계속 유효하며 일괄 migration을 요구하지 않는다.
