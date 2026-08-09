---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.checkpoint.activation-transition
revision: sha256:fdc7518e60403f6f84033cec1c7405dd8d003ae36ea95456db243c4ce0b269f2
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:a6d03d0833a14590195bdb6ec36a881b69a3a2187686179532c0f3e1e4f8a548
---
# Korean Review Projection

## Translation

# Checkpoint 활성 전환

## 선언

추적되는 active-Checkpoint record는 하나의 정확한 `CheckpointId`와 그 content hash를 가리켜야 합니다. trusted authority-basis commit과 재현 가능한 선택 lineage는 active record가 아니라 Checkpoint가 소유합니다. replacement active record는 직전 trusted active-record의 정확한 content hash도 compare-and-swap predecessor로 결합해야 합니다. 최초 activation에는 predecessor가 없어야 하고 어느 전환에도 force 경로가 있으면 안 됩니다.

Activation은 immutable Checkpoint와 active record를 함께 추가하거나 갱신하는 별도의 검토된 Git 변경으로 남아야 합니다. proposal은 authority가 아니며 accepted commit이 설정된 trusted integration ref에서 reachable할 때만 전환이 authoritative해집니다.

통합 전 gate에서 activation은 선택된 완전한 closure에 대해 `methexis.status.demotion-evidence`가 소유하는 사전 전환 demotion guard를 호출하고 그 결과를 따라야 합니다. 이 guard의 required dependency closure는 `methexis.status.negative-record`를 통해 durable negative input을 제공합니다. `invalid`, `suspect`, `stale` 결과는 prospective transition을 막아야 합니다. Guard는 상태를 낮출 수만 있고 approval이나 activation을 부여하면 안 됩니다. 이 transition은 사후 전환 계약인 `methexis.status.eligibility`에 의존하거나 이를 호출하면 안 됩니다. 최종 `active` 또는 `inactive` membership은 이 전환 이후 trusted active Checkpoint에서만 파생됩니다.

## 단계

1. 요청이 정확한 immutable Checkpoint를 식별하고 authority-basis commit이 현재 고정된 trusted commit과 같은지 검증합니다.
2. 기록된 commit에서 Checkpoint를 재현하고 lineage, byte, ID, hash, approval closure, predecessor가 하나라도 맞지 않으면 거부합니다.
3. 정확한 Checkpoint link와 compare-and-swap predecessor를 포함한 canonical active record를 만들고 검토 가능한 proposal만 게시합니다.
4. 통합 전에 선택된 완전한 closure에 대해 `methexis.status.demotion-evidence`를 호출하고 winning-condition evidence를 사용해 모든 `invalid`, `suspect`, `stale` 결과를 거부합니다.
5. repository workflow를 통해 정확한 Checkpoint 및 active-record 전환을 검토하고 통합합니다.
6. trusted integration에서 authority-basis commit이 계속 읽을 수 있고 ancestor인지 확인하고 Checkpoint를 다시 재현하며, Checkpoint를 active로 파생하기 전에 현재 승인된 required closure와 사전 전환 demotion guard가 계속 통과하는지 검증합니다. 최종 Knowledge별 eligibility는 계속 `methexis.status.eligibility`가 소유합니다.

## 완료 기준

trusted active record가 정확한 immutable Checkpoint를 가리키고, 모든 lineage 및 compare-and-swap 검사가 통과하며, accepted commit이 trusted integration에서 reachable하고, 현재 승인된 required closure가 Checkpoint를 재현하며, 사전 전환 `methexis.status.demotion-evidence` guard가 통과하고, fallback, force replacement, 부분 게시, proposal-only 상태 또는 사후 전환 eligibility dependency를 transition authority로 취급하지 않을 때만 전환이 완료됩니다.
