---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.knowledge.split-merge-supersession
revision: sha256:8593d6faf842c5e4fcece9ea1bacdeafddd7974746f365a138bf1ea67f7b8578
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:649700c37b9bb73db800ea05e4678120abc80f3a6decab88dd63ed3a6c7461e3
---
# Korean Review Projection

## Translation

# 분할 및 병합 supersession

## 선언

분할은 각각 이전 unit을 supersede하는 여러 새 `KnowledgeId`를 만들어야 합니다. 병합은 병합되는 모든 이전 unit을 supersede하는 하나의 새 `KnowledgeId`를 만들어야 합니다. 전이는 필요한 inbound relation을 하나도 미해결 상태로 남기면 안 되고, 이전 unit과 replacement를 함께 선택하면 안 되며, 사람의 의미 연속성 검토를 받아야 합니다.

## 단계

1. replacement ID를 만들고 필요한 `supersedes` edge를 기록합니다.
2. target이 active selection에서 빠지는 모든 필수 inbound relation을 해결합니다. 같은 전이에서 relation을 새 target으로 바꾸거나 그 relation의 source unit을 제거 또는 교체할 수 있습니다.
3. target 존재 여부, required graph와 supersession graph의 비순환성, 이전 unit과 replacement의 동시 선택 금지를 검증합니다.
4. 의미 mapping에 대한 사람 검토를 받고 하나의 Checkpoint 전이로 완전한 replacement selection을 활성화합니다.

## 완료 기준

모든 replacement가 의도한 안정적 identity를 가지고, 모든 필수 inbound obligation이 계속 해결되며, 구조적 guard가 모두 통과하고, 사람이 의미 연속성을 승인했으며, 하나의 active Checkpoint가 superseded unit 없이 replacement set을 선택해야 분할 또는 병합이 완료됩니다.
