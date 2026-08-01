---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.persistence.format-compatibility
revision: sha256:cf5fe8494ac485238c4f5efa9f5e6a145ab75710b4b309dea5761da91e836e6c
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:1b6e5bfc5fb46caad04d30620ba1de8ab420c38cb8b0d5ed9ae9557377649fd1
---
# Korean Review Projection

## Translation

# Session 영속 포맷 호환성

## 계약

UUIDv7만 사용하는 descriptor-aware 의미 Session Journal 포맷 `yo.semantic-journal-commit/v1`과 체크섬이 있는 물리 Session 레코드 봉투 `yo.session-record/v1`을 yo의 첫 공개 포맷 후보로 정합니다. 정확한 닫힌 구조와 UUIDv7 Session ID도 각 기준의 일부이므로, schema 태그가 같다는 이유만으로 레코드를 받아들이면 안 됩니다.

이번 초기화는 개발 단계에서 사용한 의미 포맷 `yo.semantic-journal-commit/v1`부터 `/v4`까지와 물리 포맷 `yo.session-record/v1`부터 `/v3`까지의 의미를 명시적으로 대체합니다. 의미 `/v2`, `/v3`, `/v4`, 물리 `/v2`, `/v3`, 그리고 어느 `/v1` 태그든 재사용하는 예전 숫자 ID 레코드는 의미 데이터로 받아들이기 전에 fail closed 해야 합니다. 이를 마이그레이션하거나 새 의미로 다시 해석하거나, 유효한 이력처럼 건너뛰거나, 읽을 수 있는 Session 데이터로 노출하면 안 됩니다. 복구는 승인된 호환성 계약이 명시적으로 지원하는 포맷만 읽어야 하며, 현재 기준에서는 두 가지 최신 닫힌 `/v1` 구조만 지원합니다.

이 계약은 Session Journal과 Session 레코드 영속 포맷만 다룹니다. `yo.workspace-host-id/v1`을 포함한 다른 영속 포맷은 각자의 소유 계약에서 관리합니다.

공개 전이라도 어느 `/v1` 태그 아래의 구조를 다시 교체하려면, 대체할 구조와 데이터 영향을 명시한 SOT revision을 별도로 검토해야 합니다. yo의 첫 공개 릴리스 뒤에는 이미 공개한 버전을 보존하거나 별도로 검토한 호환성·마이그레이션 계약을 제공해야 하며, 공개된 schema 태그를 조용히 초기화하면 안 됩니다.

## 이유

첫 릴리스 전에 `v1`을 다시 사용하면 실험용 번호를 호환성 부담으로 남기지 않고 공개 계약을 정직한 출발점에서 시작할 수 있습니다. 대체되는 개발 schema를 정확히 명시하고 닫힌 구조 검증을 기준의 일부로 삼으면 같은 태그를 가진 예전 레코드를 최신 데이터로 오인하지 않습니다. 하나의 공유 정책 소유자가 물리 포맷과 의미 포맷의 호환성 규칙을 함께 유지합니다.
