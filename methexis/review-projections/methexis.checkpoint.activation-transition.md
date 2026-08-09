---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.checkpoint.activation-transition
revision: sha256:e69511cf56b8562fb554eed0a1c997c6241a41b7ed42c3024d2fde9a0fccf55b
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:8c9deeebbddd64bd3edf2d5576b257b5f8595a792c078d4575c5b2524f9dbf8d
---
# Korean Review Projection

## Translation

# Checkpoint 활성 전환

## 선언

추적되는 active-Checkpoint record는 하나의 정확한 `CheckpointId`와 그 content hash를 가리켜야 합니다. trusted authority-basis commit과 재현 가능한 선택 lineage는 active record가 아니라 Checkpoint가 소유합니다. replacement active record는 직전 trusted active-record의 정확한 content hash도 compare-and-swap predecessor로 결합해야 합니다. 최초 activation에는 predecessor가 없어야 하고 어느 전환에도 force 경로가 있으면 안 됩니다.

Activation은 immutable Checkpoint와 active record를 함께 추가하거나 갱신하는 별도의 검토된 Git 변경으로 남아야 합니다. proposal은 authority가 아니며, accepted commit이 설정된 trusted integration ref에서 reachable할 때만 전환이 authoritative해집니다.

통합 전 gate에서 activation은 `SOT-006`이 소유하는 현재 Source-freshness guard를 호출하고 그 결과를 따라야 합니다. Guard의 입력, eligibility 상태, 우선순위, demotion evidence, context 제외, 실패 의미는 계속 `SOT-006`만 소유하며 이 procedure는 이를 다시 정의하지 않습니다. 통과하지 못한 결과는 prospective transition을 막아야 합니다. 통합 후에도 외부에서 소유하는 그 guard가 통과할 때만 trusted Checkpoint를 `active`로 파생해야 합니다.

## 단계

1. 요청이 정확한 immutable Checkpoint를 식별하고 authority-basis commit이 현재 고정된 trusted commit과 같은지 검증합니다.
2. 기록된 commit에서 Checkpoint를 재현하고 lineage, byte, ID, hash, approval closure, predecessor가 하나라도 맞지 않으면 거부합니다.
3. 정확한 Checkpoint link와 compare-and-swap predecessor를 포함한 canonical active record를 만들고 검토 가능한 proposal만 게시합니다.
4. 통합 전에 선택된 완전한 closure에 대해 현재 `SOT-006` Source-freshness guard를 호출하고, 그 authority에 따라 통과하지 못한 모든 결과를 거부합니다.
5. repository workflow를 통해 정확한 Checkpoint 및 active-record 전환을 검토하고 통합합니다.
6. trusted integration에서 authority-basis commit이 계속 읽을 수 있고 ancestor인지 확인하고, Checkpoint를 다시 재현하며, active 상태를 파생하기 전에 현재 승인된 required closure와 Source freshness가 계속 일치하는지 검증합니다.

## 완료 기준

trusted active record가 정확한 immutable Checkpoint를 가리키고, 모든 lineage 및 compare-and-swap 검사가 통과하며, accepted commit이 trusted integration에서 reachable하고, 현재 승인된 closure가 Checkpoint를 재현하며, 필요한 `SOT-006` Source-freshness guard가 통과하고, fallback·force replacement·부분 게시·proposal-only 상태를 authority로 취급하지 않을 때만 전환이 완료됩니다.
