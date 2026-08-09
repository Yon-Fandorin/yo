---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.source.kind-vocabulary
revision: sha256:0fb2758aa897d526a4d6b4bc4f5d56c080725dca7192026e8a855b3a4e020585
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:4383943f9f922e2b5f0ed5a8be9f22fa32d4ba9e2566f863932e26f92bebd1dc
---
# Korean Review Projection

## Translation

# Source 종류 어휘

## 선언

초기 Source 어휘는 다음 네 종류로 닫혀 있습니다.

- `decision`은 수락된 설계 결정을 기록합니다.
- `code`는 저장소 경로, symbol, 정확한 content hash를 기록합니다.
- `conversation`은 허가된 최소 발췌문 또는 opaque reference를 기록합니다.
- `external`은 저장소 외부의 문서 또는 표준을 기록합니다.

Conversation material은 허가된 발췌문 또는 content hash가 있는 opaque reference 중 하나여야 합니다. External freshness는 immutable, mutable, attested 중 하나로 선언해야 합니다. 해당 종류와 freshness mode의 verifier가 존재할 때까지 Conversation과 External Source는 ineligible 상태로 남아야 합니다.
