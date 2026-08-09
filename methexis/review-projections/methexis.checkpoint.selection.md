---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.checkpoint.selection
revision: sha256:d99f103d885b6d08f23043c162b5ede400e0c2d2f08664a5af5fab957d9320f0
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:7395ee5a2e6565670619881a4e1ac8502da511c60918f442cee07206c88e6d72
---
# Korean Review Projection

## Translation

# 체크포인트 선택

## 선언

`Checkpoint`는 승인된 `KnowledgeId`에서 정확한 `RevisionId`로 이어지는 일관된 맵을 고정해야 합니다. 요청은 하나 이상의 명시적 root를 지정해야 합니다. 선택은 모든 root와 `depends_on` 및 `constrained_by`의 완전한 전이 closure를 포함해야 하며, `validated_by`와 `applies_to`는 unit을 선택하면 안 됩니다.

선택되는 모든 revision에는 정확한 trusted approval이 있어야 합니다. root나 필수 dependency가 없거나 구성원 하나라도 승인되지 않았다면 부분 Checkpoint를 만들지 않고 선택에 실패해야 합니다. replacement와 그 replacement가 `supersedes`하는 unit을 함께 선택하면 안 됩니다.

Checkpoint는 역사적 입력 marker인 `source_status: not_evaluated`를 유지해야 합니다. Source freshness와 그 결과인 active 또는 degraded 상태는 현재 시점에 파생되는 guard이며 Checkpoint 선택 상태에 기록하면 안 됩니다.
