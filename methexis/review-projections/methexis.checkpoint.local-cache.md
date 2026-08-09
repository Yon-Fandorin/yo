---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.checkpoint.local-cache
revision: sha256:4303bf26d4f7506abbda0e203646ab10e8de0f3a925ebbd0ef25c110d5cf3cbc
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:17dd7b2785e9a3f61c5bf460b38c20ca42b75016fe3aa421e5d326edd509285f
---
# Korean Review Projection

## Translation

# 로컬 active-Checkpoint 캐시

## 선언

모든 local active pointer는 재구성 가능한 비권위 캐시여야 합니다. 이 캐시는 파생의 근거가 된 정확한 Git tree identity와 trusted active-Checkpoint hash를 결합하고, crash-safe하게 교체하며, 어느 identity든 일치하지 않으면 사용하지 말고 폐기해야 합니다.

로컬 캐시는 approval, activation, eligibility를 부여하면 안 되고 추적되는 active record를 대체하면 안 됩니다. 동시에 일어나는 authority 변경은 repository merge와 review로 직렬화되며, runtime database lock을 authority boundary로 제시하면 안 됩니다.
