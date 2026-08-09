---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.knowledge.revision-identity
revision: sha256:b8de31ffaf360ef62cc6a8cc67d6b32116153b5fb4a6738a1ac28524ef4b6457
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:c2bca3b10b47dd2e18dea2939623630fba4a7580ea8bbae5add76993135002b8
---
# Korean Review Projection

## Translation

# Knowledge revision identity

## 선언

`RevisionId`는 하나의 안정적인 KnowledgeId가 가진 정확한 canonical meaning을 식별합니다. schema version, KnowledgeId, kind, owner, canonical body, 정렬된 정확한 Source reference, 그리고 target reference가 정렬된 각 closed relation type을 포함하는 하나의 모호하지 않은 length-delimited 의미 표현에 대한 `sha256:<lowercase-hex>`로 인코딩해야 합니다. target이 없는 relation도 빈 목록으로 포함해야 합니다.

loader는 hash를 계산하기 전에 CRLF와 bare CR 줄바꿈을 LF로 정규화해야 합니다. 물리 path, YAML key 순서 또는 formatting, generation time, 원래 line-ending 표현은 RevisionId에 영향을 주면 안 됩니다. 그 밖의 모든 canonical body byte는 의미를 유지해야 합니다.
