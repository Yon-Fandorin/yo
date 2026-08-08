---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.persistence.format-compatibility
revision: sha256:acc24661fcc92c3bc78232ce7bf8ea0d59aae3a5e0c8e544d168248dc62d0e90
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:fb0dbcaef2ca1459cb85051f80feba912db4a5fe401437fff4b43f26bacd64b1
---
# Korean Review Projection

## Translation

# Session 영속 포맷 호환성

## 계약

UUIDv7만 사용하는 descriptor-aware 의미 Session Journal
`yo.semantic-journal-commit/v1`과 체크섬이 있는 물리 Session 레코드
`yo.session-record/v1`을 yo의 첫 공개 포맷 후보로 유지합니다. 첫 공개 릴리스 전에 첫 reviewed revision이 structured-input 의미 `/v1`을
anchored-session development shape로 교체했고, 두 번째 reviewed revision은 이를
replay-delta development shape로 교체했습니다. 이번 세 번째 reviewed revision은
그 바로 앞 shape를 아래의 닫힌 anchored-session 의미 `/v1`로 교체합니다. 정확한 구조와 UUIDv7 Session ID까지 기준에 포함하며 schema
태그가 같다는 이유만으로 레코드를 받아들이지 않습니다.

Descriptor만 있는 commit을 포함한 모든 의미 `/v1` commit은 top-level에 정확히
`format: anchored-session`을 가져야 합니다. 이 값이 없거나 알 수 없거나 한 Session
이력에 서로 다른 format 세대가 섞이면 의미 데이터로 받아들이기 전에 fail closed
합니다.

저장되는 `command_committed`, `event_committed`와 여섯 correlation record는 각각
필수 양의 `journal_sequence`를 가집니다. 이 값은 유일한 Session Journal writer가
semantic commit 시점에 부여하며 codec, repository, retry, snapshot, remote transport는
새 값을 만들거나 번호를 다시 매기지 않습니다. 반대로 `session_descriptor`,
`message_reset`, `message_segment`, `message_ended`는 이 field를 nullable로 두지 않고
구조적으로 갖지 않습니다. 명시적인 sequence는 한 Session에서 유일하고 엄격히
증가하지만, 여러 live text update가 하나의 segment로 정규화될 수 있으므로 연속일
필요는 없습니다. Descriptor가 아닌 commit의 양의 `journal_cutoff`는 뒤로 가지 않고
그 commit 안의 모든 명시적인 sequence 이상이어야 합니다.
새 incremental commit이 도입하는 모든 sequence는 직전의 durable `journal_cutoff`보다
커야 합니다. Complete snapshot은 incremental commit이 아니며 자신이 대체하는 prefix의
정확한 sequence 값만 다시 기록할 수 있습니다. Recovery는 sequence를 그대로 보존하고
빈 번호를 채우거나 delta 개수를 추측하거나 번호를 다시 매기지 않으며, snapshot도 값과
cutoff를 정확히 보존합니다. Recovery는 이 정보로 메모리 안의
`JournalSequence -> semantic record` 인덱스를 다시 만들되 인덱스 자체는 저장하지
않습니다. 중복·역순 sequence, 직전 cutoff 안에 새로 들어온 incremental sequence,
현재 cutoff를 넘는 sequence, 뒤로 간 cutoff, 없거나 종류가 틀린 참조는 fail closed
합니다. ReplaySequence는 semantic payload 안의 normalized record를 정렬하는 내부
좌표이고, 별도의 RepositorySequence가 물리 Session record append 순서를 나타냅니다.

기존 structured input 계약은 그대로 포함됩니다. `StartTurn`과 `SteerTurn` 명령은
canonical UUIDv4 `submission_id`와 `input` 객체를 가지며, input은 정확히
`profile: yo.structured-input/v1`, 제출된 UTF-8 `text`, 순서가 있는 typed
`references` 배열을 포함합니다. 상관관계가 있는 Activity 사용자 입력 응답은 별도
SubmissionId 없이 같은 input 구조를 사용합니다. 의미 재생은 명령 종류, 대상 Turn,
SubmissionId, `UserInput`을 하나의 수락된 submission으로 보존하고, 한 Session에서
같은 SubmissionId의 두 번째 commit을 거부합니다. SubmissionId는 내부 상관관계이며
별도 표시 계약 전에는 일반 Chat과 Transcript에 노출하지 않습니다.

각 reference occurrence는 `text`를 가리키는 unsigned 64-bit 반개구간 UTF-8 byte
offset `start`·`end`, capture 당시의 정확한 `projection`, typed identity를
보존합니다. 구간은 비어 있지 않고 UTF-8 경계에 있으며 엄격한 비중첩 순서를
지켜야 하고, projection은 해당 text bytes와 같아야 합니다. Replay는 보이는
`@path`나 `$name`을 다시 파싱하지 않습니다. Workspace occurrence는 identity,
execution environment, workspace, root, 정규화한 relative path, file 또는 directory
kind를 보존합니다. Skill occurrence는 identity, execution environment, locator, name,
workspace·user·system·admin scope, 양의 catalog generation, entry revision을
보존하며 최대 하나만 허용합니다. 알 수 없는 field·tag·kind·scope, 빈 metadata,
0 generation, 잘못된 root-relative path와 profile은 fail closed 합니다.

새 의미 `/v1`은 payload를 담지 않는 일반 exchange record 하나와 재개 전용 record
다섯 종류를 추가합니다.

- `backend_exchange_observed`: 양의 `epoch`, UUIDv4 `operation_id`, exchange 종류와
  방향, payload schema, 선택적인 상관 sequence와 backend identity, Request Audit
  detail의 가용 상태
- `backend_binding_opened`: 양의 `epoch`, `backend_kind`, `backend_version`,
  versioned binding·model·Session locator identity와 전환 정보
- `backend_binding_closed`: 닫는 양의 `epoch`와 `replaced`, `revoked`, `exhausted` 중
  하나인 이유
- `backend_request_accepted`: 양의 `epoch`·`turn_id`, 수락된 SubmissionId와 같은
  canonical UUIDv4 `operation_id`, matching outbound exchange sequence,
  `schema`와 `value`로 된 `request_identity`
- `backend_resumable_outcome`: 양의 `epoch`·`turn_id`·`accepted_request_sequence`,
  정확한 `status: completed`, backend가 제공할 때만 존재하는 `outcome_identity`
- `continuation_anchor`: 양의 `epoch`·`accepted_request_sequence`·
  `resumable_outcome_sequence`·`journal_boundary`

Exchange 종류는 request, response, notification, server request, retry, terminal
outcome을 구분하고 방향은 yo에서 backend 또는 backend에서 yo를 구분합니다. Detail
가용 상태는 persisted, volatile, missing, unsupported, unpersisted, redacted 중
하나입니다. 각 operation의 첫 exchange가 Session에서 유일한 operation ID를 가집니다.
`StartTurn`·`SteerTurn`에서 나온 request는 SubmissionId를 사용하고, 그 밖의 request,
notification, server request는 writer가 UUIDv4를 부여합니다. Backend ID가 있으면 별도
`exchange_identity`에 저장합니다.

Root request·notification·server request는 상관 sequence가 없습니다. 이후 exchange는
참조한 앞 exchange와 같은 operation ID를 사용합니다. Response는 반대 방향의 request
또는 server request만, retry는 같은 방향의 request·server request·retry만,
terminal outcome은 같은 operation chain의 request·server request·retry·response만
가리킬 수 있습니다. Notification은 상관 edge를 만들지 않습니다. 모든 edge는 같은
epoch 안에서 뒤에서 앞으로만 향하고 하나의 operation ID로 두 번째 root를 만들 수
없습니다. 이 record 자신의 JournalSequence가 관찰 경계이므로 payload detail이 없어도
각 흐름이 섞이지 않습니다.

Binding, model, Session locator와 각 exchange·request·outcome identity는 `schema`와
`value`로 된 versioned opaque 객체입니다. `payload_schema`, `backend_kind`, 모든
identity schema는 비어 있지 않은 최대 128-byte ASCII 문자열입니다.
`backend_version`과 locator·identity value는 비어 있지 않은 최대 4096-byte UTF-8
문자열입니다. 공통 codec은 닫힌 구조, 크기, 순서, record 관계만 검증하고 각 adapter가
값의 해석과 native resume 때 binding identity 비교를 소유합니다.

Binding 전환에는 initial, exact replay, lossy handoff 중 하나와 cache 상태, 선택적인
source Anchor sequence를 기록합니다. Initial에는 source가 없고 cache가 해당 없음이며,
두 replacement 방식은 닫힌 앞 epoch의 Anchor를 요구합니다. Exact replay는 cache
손실을 기록하고 lossy handoff는 cache 손실 또는 알 수 없음을 기록하면서 눈에 보이는
context-loss 경계가 됩니다.

첫 binding epoch는 backend Session 생성과 matching `SessionCreated` commit 뒤에 1로
열리고 이후 정확히 1씩 증가합니다. 동시에 하나만 열리며 replacement는 기존 epoch를
`replaced`로 닫고 다음을 엽니다. 프로세스 종료나 재시작은 열린 epoch를 닫지 않습니다.
Native resume은 새 binding-open record를 만들지 않고 기록된 binding identity를 먼저
검증합니다. 불일치하면 기존 epoch를 닫고 허용된 replacement epoch를 열기 전에는 다음
request를 수락할 수 없습니다. Accepted request는 matching `StartTurn` 또는
`SteerTurn`과 outbound request exchange 뒤에 기록하며, command·exchange·accepted
record의 `operation_id`는 모두 같은 SubmissionId입니다. 하나의 Turn에 여러 submission이
수락될 수 있지만 완료 outcome은 같은 epoch와 Turn에서 가장 최근 accepted request를
참조해야 합니다.

`backend_resumable_outcome`은 matching semantic `TurnFinished(completed)` 뒤에만
유효합니다. Backend가 별도의 안정된 결과 identity를 제공하면 `outcome_identity`에
저장하고, 제공하지 않으면 명시적으로 생략한 채 참조한 accepted request identity를
backend operation identity로 유지합니다. Writer는 가짜 결과 ID를 만들지 않습니다.
실패하거나 중단된 Turn은 resumable outcome과 Anchor를 만들지 않습니다.

`continuation_anchor`는 참조한 outcome 바로 뒤에 같은 semantic commit으로 기록합니다.
여섯 record의 모든 sequence 참조는 storage ReplaySequence가 아니라 semantic
JournalSequence입니다. Request와 outcome sequence는 같은 epoch의 상관 record를
가리키고 `journal_boundary`는 outcome의 JournalSequence와 같아야 합니다. Anchor
record 자신의 JournalSequence가 물리 discovery metadata에 투영됩니다. 따라서 완료된
Turn, resumable outcome, Anchor가 그 순서로 하나의 물리 append가 되면서도 Anchor가
자기 자신을
완료 경계라고 순환해서 주장하지 않습니다. Recovery와 snapshot은 전체 binding·
correlation graph를 보존하고 다시 검증하며, 완료 Turn·discovery summary·backend wire
payload·Request Audit detail에서 Anchor를 추측해 만들지 않습니다.

모든 물리 `/v1` record는 전체 Session descriptor, writer가 지정한
`updated_unix_millis`, 선택적인 binding epoch, 선택적인 최신 유효 Anchor
`JournalSequence`를 discovery 객체에 포함합니다. 기존 CRC32C가 schema, Session ID,
RepositorySequence, record kind, 정확한 payload와 discovery 전체를 함께 묶습니다.

이번 초기화는 `format: anchored-session`이 없는 바로 앞의 structured-input과
string-input 의미 `/v1`, summary 없는 물리 `/v1`, 개발 단계 semantic `/v1`부터
`/v4`, physical `/v1`부터 `/v3`, 숫자 identity를 사용한 옛 `/v1`을 대체합니다.
새 닫힌 구조가 아닌 개발 데이터는 migration, dual reader, compatibility shim 없이
fail closed 합니다. 현재 checksummed physical envelope 자체는 정확한 새 payload를 이미
묶을 수 있으므로 바꾸지 않습니다. 공개 전 같은 `/v1`을 다시 교체하려면 데이터 영향을
수용하는 별도 SOT 검토가 필요하고, 첫 공개 뒤에는 공개 버전을 보존하거나 명시적인
호환성·migration 계약을 제공해야 합니다.


이번 revision은 public release 전 anchored-session semantic Journal /v1을 두 번째로 명시적으로 교체합니다. 기존 backend correlation record는 payload-free 상태를 유지하고, exact provider-neutral replay는 별도 model_replay_delta record가 소유합니다. model_replay_delta는 TurnFinished(completed) 뒤, backend_resumable_outcome과 continuation_anchor 앞에 같은 semantic commit으로 기록되며 outcome은 replay delta의 JournalSequence를 참조합니다. Replay contract는 system prompt와 ordered tool name, description, schema version, closed schema를 보존하고 replay item은 message, function call, function result의 정확한 순서와 관계를 보존합니다. Contract는 1 MiB, delta는 16 MiB, Anchor가 선택한 prefix는 64 MiB 및 4096 item으로 제한하며 초과하면 completed지만 non-resumable인 Turn과 explicit context exhaustion이 됩니다. Silent truncation이나 implicit compaction은 허용하지 않습니다. Persisted failed outcome은 required nullable code와 message를 모두 가지며 tool validation은 stable yo.tool.validation.*/v1 code를 사용합니다. Argument와 output은 Activity, 후속 model input, replay persistence 전에 semantic redaction admission을 통과해야 합니다. 이 교체 전 development shape는 같은 schema tag라도 fail closed합니다.


이번 세 번째 reviewed pre-release revision은 바로 앞 replay-delta development shape를 교체합니다. backend_binding_opened는 continuation_strategy를 명시하며 exact_replay는 local_client 또는 managed_server executor를 갖고 backend_managed_state는 executor를 금지합니다. 이는 새 epoch의 seed 방법을 뜻하는 transition.mode와 별개입니다. exact replay binding에서만 model_replay_delta와 outcome의 replay_delta_sequence가 필수이며 delta가 outcome 바로 앞에 있어야 합니다. backend-managed binding에서는 둘 다 금지되고 TurnFinished(completed), payload-free outcome, Anchor가 같은 commit에 연속해서 기록됩니다. 두 exact replay executor의 replay contract, bounds, digest, ordering, Anchor validation은 동일하며 request를 조립하는 위치만 다릅니다. managed_server는 reviewed remote repository 구현 전에는 현재 implementation이 기록할 수 없는 예약 값입니다. 같은 `/v1` tag를 사용한 직전 development shape는 fail closed합니다.

## 이유

첫 릴리스 전 `/v1`을 다시 닫으면 실험 번호를 공개 호환성 부담으로 남기지 않습니다.
Binding, accepted request, resumable outcome을 별도 record로 보존하면 Codex처럼 요청과
결과 ID가 같은 backend와 Kimi처럼 다를 수 있는 backend를 같은 의미 계약으로 다룰 수
있습니다. Anchor는 문자열을 복사하지 않고 앞 record의 JournalSequence를 참조하므로
작고 검증 가능하며, 기존 물리 envelope checksum만으로 새 의미 payload까지 보호합니다.
