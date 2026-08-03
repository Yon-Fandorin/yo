---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.persistence.format-compatibility
revision: sha256:3ae1182cac32e14286e340fdbb41373d9c26106a64b1fe1f7b4d56aaed7a61a1
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:2227cd7267de6e8d0a6da6d1aac68ce78f80718d06e9bd6d9bc1e180798dfe0a
---
# Korean Review Projection

## Translation

# Session 영속 포맷 호환성

## 계약

UUIDv7만 사용하는 descriptor-aware 의미 Session Journal `yo.semantic-journal-commit/v1`과 체크섬이 있는 물리 Session 레코드 `yo.session-record/v1`을 yo의 첫 공개 포맷 후보로 정합니다. 첫 공개 릴리스 전인 지금, 바로 앞의 문자열 입력 의미 `/v1`을 아래의 닫힌 구조화 입력 의미 `/v1`로 교체합니다. 정확한 구조와 UUIDv7 Session ID까지 기준에 포함하며, schema 태그가 같다는 이유만으로 레코드를 받아들이지 않습니다.

Descriptor만 있는 commit을 포함해 모든 의미 `/v1` commit은 top-level에 정확히 `format: structured-input`을 가져야 합니다. 이 값이 없거나 알 수 없거나 한 Session 이력에 서로 다른 format 세대가 섞이면 의미 데이터로 받아들이기 전에 fail closed 합니다.

`StartTurn`과 `SteerTurn` 명령 레코드는 canonical UUIDv4 문자열인 `submission_id`와 `input` 객체를 가져야 합니다. 상관관계가 있는 Activity 사용자 입력 응답은 request identity가 이미 연결을 소유하므로 별도 SubmissionId 없이 같은 `input` 객체를 사용합니다. 닫힌 input 객체는 정확한 `profile: yo.structured-input/v1`, 제출된 정확한 UTF-8 문자열인 `text`, 0개 이상의 타입화된 occurrence를 순서대로 담는 `references` 배열로 구성됩니다.

의미 재생 domain은 committed submission을 명령 종류(`StartTurn` 또는 `SteerTurn`), 대상 Turn, SubmissionId, `UserInput`으로 함께 보존해야 합니다. 이 레코드 자체가 정확한 submission이 수락되었다는 증거입니다. Recovery와 snapshot은 그 상관관계를 유지해야 합니다. 각 SubmissionId는 한 Session에서 committed submission 레코드 하나만 식별할 수 있고, 나머지 field가 byte 단위로 같더라도 두 번째 committed occurrence는 유효하지 않습니다. SubmissionId는 내부 상관관계이며 추후 표시 계약이 명시적으로 선택하기 전에는 일반 Chat이나 Transcript에 노출하지 않습니다.

각 occurrence는 `text`를 가리키는 반개구간 UTF-8 byte offset `start`와 `end`, live capture에서 수락한 정확한 `projection`을 포함합니다. Offset은 unsigned 64-bit 범위의 JSON 정수이고 decoder가 주소로 표현할 수 없으면 거부합니다. 구간은 비어 있지 않고 UTF-8 경계에 있으며 겹치지 않는 엄격한 순서를 지켜야 합니다. `projection`은 비어 있지 않고 해당 `text` 구간과 byte 단위로 같아야 합니다. Live writer는 commit 전에 이 projection과 typed reference의 일치를 검증해야 합니다. Replay에서는 projection 문자열이 아니라 typed identity가 권위이며 보이는 `@path`나 `$name`을 파싱해 identity를 복구하면 안 됩니다. 미래의 표시 정책 변경은 새 capture에만 적용되고 저장된 projection bytes를 다시 해석하지 않습니다.

`workspace` occurrence는 정확히 `type: workspace`, `start`, `end`, `projection`, `identity`, `execution_environment_identity`, `workspace_identity`, `root_identity`, `relative_path`, `kind`를 포함합니다. `kind`는 `file` 또는 `directory`입니다. `skill` occurrence는 정확히 `type: skill`, `start`, `end`, `projection`, `identity`, `execution_environment_identity`, `locator`, `name`, `scope`, `catalog_generation`, `entry_revision`을 포함합니다. `scope`는 `workspace`, `user`, `system`, `admin` 중 하나이고 `catalog_generation`은 양의 unsigned 64-bit JSON 정수입니다.

변경 불가능한 `yo.structured-input/v1` profile은 occurrence에 해당하는 모든 identity, execution-environment identity, workspace identity, root identity, locator, name, entry revision이 비어 있지 않을 것을 요구합니다. Workspace `relative_path`는 비어 있지 않은 root-relative `/` 구분 경로이고, 앞뒤 `/`나 빈 component, `.`, `..` component를 허용하지 않습니다. Skill occurrence는 최대 하나입니다. 알 수 없는 input·occurrence field, tag, kind, scope, 0인 skill generation, 잘못된 metadata·profile 값은 fail closed 합니다. 이후 live domain 규칙 변경이 저장된 `/v1` decoder를 암묵적으로 바꾸면 안 됩니다.

모든 물리 `/v1` 레코드는 전체 UUIDv7 Session ID, workspace-host identity, host-normalized workspace path, start time으로 이루어진 Session descriptor, writer가 지정한 `updated_unix_millis`, 선택적인 binding epoch, 선택적인 최신 유효 Continuation Anchor `JournalSequence`를 담은 `discovery` 객체를 가져야 합니다. 기존 CRC32C 하나가 schema, Session ID, `RepositorySequence`, kind, 정확한 payload bytes와 함께 discovery 전체를 같은 checksum preimage로 묶으며 두 번째 checksum이나 append를 만들지 않습니다.

이번 초기화는 필수 `format: structured-input`이 없는 모든 의미 `/v1`을 대체하며, 여기에는 문자열 입력 의미 `/v1`도 포함됩니다. 또한 summary 없는 물리 `/v1`, 개발 단계 의미 `/v1`부터 `/v4`, 물리 `/v1`부터 `/v3`, 숫자 Session ID를 사용한 옛 `/v1`을 대체합니다. 새 닫힌 구조가 아닌 개발 데이터는 의미 데이터로 받아들이기 전에 fail closed 하며 migration, dual reader, compatibility shim, 옛 wire model을 남기지 않습니다. 대체된 구조가 거부되는지 증명하는 최소 fixture만 남길 수 있습니다. 현재 복구가 지원하는 것은 이 계약이 명시한 최신 의미 `/v1`과 물리 `/v1`뿐입니다.

현재 체크섬이 있는 물리 `yo.session-record/v1` envelope는 정확한 의미 payload bytes를 이미 CRC32C로 묶으므로 바꾸지 않습니다. 이 계약은 Session Journal과 Session 레코드 영속화만 다룹니다. `yo.workspace-host-id/v1` 같은 다른 영속 포맷은 각 소유 계약을 따릅니다. 공개 전이라도 같은 태그를 다시 교체하려면 데이터 영향을 수용하는 새 SOT 검토가 필요합니다. 첫 공개 릴리스 뒤에는 공개 버전을 보존하거나 명시적으로 검토한 호환성·migration 계약을 제공해야 합니다.

## 이유

첫 릴리스 전 `v1` 재사용은 실험 번호를 공개 호환성 부담으로 만들지 않습니다. 모든 commit의 format 판별자는 같은 태그를 가진 옛 레코드까지 확실히 거부합니다. SubmissionId, capture 당시 projection, typed reference occurrence를 수집 시점에 보존하면 실행된 입력을 표시 문자열에서 추측하거나 미래 표시 정책으로 다시 해석하지 않고 재생할 수 있습니다. 기존 물리 envelope checksum은 별도의 저장 권위 없이 새 의미 payload도 그대로 보호합니다.
