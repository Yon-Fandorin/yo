---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.projection.korean-review
revision: sha256:3547762eb57358842848709eecef8c28378d0a90e82328aa239cbf0b409633c8
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:127e57f6eaf6c16d454a7fc1ef2d3febf64d960efea82e9e33fe067b9b32a3ca
---
# Korean Review Projection

## Translation

# 한국어 review Projection

## 선언

Pilot은 `methexis/review-projections/` 아래에서 `KnowledgeId`마다 생성된 한국어 review Projection 하나를 유지해야 합니다. Projection은 정확한 `RevisionId`, Projection profile, compiler identity, 결정론적 request lineage, 실제로 검토한 파일의 정확한 byte를 바인딩해야 합니다.

직접 편집, revision drift, lineage drift는 구조적 실패여야 합니다. 파일을 암묵적으로 또는 직접 고치는 대신 명시적 request로 다시 생성해야 합니다.
