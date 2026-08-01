---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.persistence.format-compatibility
revision: sha256:81ddecfb3c16d2c61cabcfd9c8f21bc2c422aa269a5e390088abc9b78d53a112
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:3fc61139621b4b2c46a039befcb41de13d478e56084d7c4ba21e2ff22e4b65de
---
# Korean Review Projection

## Translation

# Session 영속 포맷 호환성

## 계약

UUIDv7만 사용하는 descriptor-aware 의미 Session Journal `yo.semantic-journal-commit/v1`과 체크섬이 있는 물리 Session 레코드 `yo.session-record/v1`을 첫 공개 포맷 후보로 정합니다. 의미 `/v1` 구조는 바꾸지 않습니다. 첫 공개 릴리스 전인 지금, 바로 앞에서 사용하던 summary 없는 물리 `/v1` 구조를 새로운 닫힌 물리 `/v1` 구조로 교체합니다. 같은 schema 태그만으로 레코드를 받아들이지 않으며, 정확한 구조와 UUIDv7 Session ID까지 기준에 포함합니다.

새 물리 `/v1` 레코드는 모두 `discovery` 객체를 가져야 합니다. 여기에는 전체 UUIDv7 Session ID, workspace-host identity, host-normalized workspace path, start time으로 이루어진 Session descriptor와 writer가 지정한 `updated_unix_millis`, 선택적인 binding epoch, 선택적인 최신 유효 Continuation Anchor `JournalSequence`가 들어갑니다. 기존 CRC32C 하나가 schema, Session ID, `RepositorySequence`, kind, 정확한 payload bytes와 함께 discovery 전체를 명시적인 checksum preimage로 묶습니다. 두 번째 checksum이나 append를 만들지 않습니다.

이번 초기화는 summary 없는 바로 전 물리 `/v1`, 개발 단계 의미 `/v1`부터 `/v4`, 물리 `/v1`부터 `/v3`, 숫자 Session ID를 사용한 옛 `/v1`을 대체합니다. 새 닫힌 구조가 아닌 개발 데이터는 의미 데이터로 받아들이기 전에 fail closed 하며 migration, dual reader, compatibility shim, 옛 wire model을 남기지 않습니다. 대체된 구조가 거부되는지 증명하는 최소 fixture만 남길 수 있습니다. 현재 복구가 지원하는 것은 최신 의미 `/v1`과 최신 물리 `/v1`뿐입니다.

이 계약은 Session Journal과 Session 레코드만 다룹니다. `yo.workspace-host-id/v1` 같은 다른 영속 포맷은 각 소유 계약을 따릅니다. 공개 전이라도 `/v1`을 다시 교체하려면 대체 구조와 데이터 영향을 명시한 새 SOT 검토가 필요합니다. 첫 공개 릴리스 뒤에는 공개 버전을 보존하거나 명시적으로 검토한 호환성·migration 계약을 제공해야 합니다.

## 이유

첫 릴리스 전 `v1` 재사용은 실험 번호를 공개 호환성 부담으로 만들지 않습니다. 대체 대상을 명시하고 닫힌 구조를 검증하면 같은 태그의 옛 레코드를 최신 데이터로 오인하지 않습니다. Discovery를 기존 envelope checksum에 포함하면 별도 권위 없이 검증된 마지막 레코드 하나로 bounded discovery를 수행할 수 있습니다.
