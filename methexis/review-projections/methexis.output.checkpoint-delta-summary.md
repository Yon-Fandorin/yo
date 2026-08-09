---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.output.checkpoint-delta-summary
revision: sha256:ececfd2c5aa339f7aa40e2a1bbf6a90fc37356664539a697afea62bce11582ad
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:410a61b30374aeb96de65cb7af2c836adfc8aeb6a1434533b7fbd733804ae60c
---
# Korean Review Projection

## Translation

# Checkpoint 변경분 성공 요약

## 선언

성공한 `create-checkpoint` 또는 `propose-activation` 작업은 candidate Checkpoint를 같은 고정 trusted snapshot에서 포착한 활성 Checkpoint와 비교해야 합니다. 성공 결과는 해당 trusted commit, candidate Checkpoint ID와 hash, candidate 산출물 경로, 그리고 기준 활성 Checkpoint ID와 hash 또는 기준이 없다는 명시적 표시를 식별해야 합니다.

존재 여부나 RevisionId가 달라진 각 KnowledgeId에 대해 결과는 KnowledgeId 순으로 정렬된 항목 하나를 포함하고, 변경 전 RevisionId 또는 부재와 변경 후 RevisionId 또는 부재를 기록해야 합니다. 존재 여부가 달라진 각 root에 대해서도 root 순으로 정렬된 항목 하나에 변경 전후 존재 여부를 기록해야 합니다. 또한 candidate 필수 closure의 전체 unit 수와 두 Checkpoint에서 KnowledgeId와 RevisionId가 모두 같은 unit 수를 보고해야 합니다. 활성 Checkpoint가 없으면 candidate의 모든 KnowledgeId와 root는 추가이며 동일 unit 수는 0입니다.

기본 성공 결과는 변경되지 않은 closure 항목이나 선택 사유를 반복해서는 안 됩니다. 변경 불가능한 candidate Checkpoint 산출물이 전체 root, unit, revision, 사유를 무결성이 고정된 상태로 소유합니다. 실패 결과는 문제 진단과 복구에 필요한 영향 식별자를 모두 유지해야 하며, 변경분 우선 성공 보고가 실패 근거를 줄여서는 안 됩니다.
