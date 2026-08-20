---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.persistence.format-compatibility
revision: sha256:0984d08af628f5dd1348615e8331836f1612cc837924ad9fd670f878993d9f22
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:ff63ffb7b9f697c2f99015ba9a8d03caa87c102049ba7287441cae1bb86d9f2d
---
# Korean Review Projection

## Translation

# Session 영속 포맷 호환성

## 계약

UUIDv7만 사용하는 descriptor-aware 의미 Session Journal
`yo.semantic-journal-commit/v1`과 체크섬이 있는 물리 Session 레코드
`yo.session-record/v1`을 yo의 첫 공개 포맷 후보로 유지합니다. 첫 공개 릴리스 전에 첫 reviewed revision이 structured-input 의미 `/v1`을
anchored-session development shape로 교체했고, 두 번째 reviewed revision은 이를
replay-delta development shape로 교체했습니다. 세 번째 reviewed revision은 그 shape를
continuation strategy를 명시하는 anchored-session shape로 교체했습니다. 이번 네 번째
reviewed revision은 같은 shape에 아래의 optional assistant-refusal replay field를 명시적으로
확장했습니다. 이번 다섯 번째 reviewed revision은 물리 v1 envelope를 유지하면서 같은 공개 전 semantic shape에 provider-private replay item 하나와 replay-profile evidence를 additive하게 확장합니다. 정확한 구조와 UUIDv7 Session ID까지 기준에 포함하며 schema
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
다섯 종류를 추가합니다. `model_replay_delta`의 ordered non-empty list는 exact replay item을 담으며, 기존 message, function call, function result 외에 아래에서 정의하는 provider-private assistant item도 포함할 수 있습니다.

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

닫힌 `transition` object는 exact `mode: initial`, `exact_replay`, `lossy_handoff` 중 하나와 exact `cache: not_applicable`, `lost`, `unknown` 중 하나, 선택적인 양의 `source_anchor_sequence`를 포함합니다. `initial`은 `cache: not_applicable`과 source Anchor 부재를 요구합니다. 두 replacement mode는 모두 앞서 닫힌 epoch의 source Anchor를 요구합니다. `exact_replay`는 `cache: lost`를 요구합니다. `lossy_handoff`는 `cache: lost` 또는 `unknown`을 요구하고 binding open을 눈에 보이는 context-loss 경계로 표시합니다. 사용자가 승인한 transformed-context 설명은 opaque backend identity가 아니라 일반 semantic Journal data로 남습니다. 따라서 binding의 backend·model identity, transition mode, source Anchor, cache state는 Request Audit 상세 없이도 계속 사용할 수 있습니다.

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


이번 revision은 public release 전 anchored-session semantic Journal /v1을 두 번째로 명시적으로 교체합니다. 기존 backend correlation record는 payload-free 상태를 유지하고, exact provider-neutral replay는 별도 model_replay_delta record가 소유합니다. model_replay_delta는 TurnFinished(completed) 뒤, backend_resumable_outcome과 continuation_anchor 앞에 같은 semantic commit으로 기록되며 outcome은 replay delta의 JournalSequence를 참조합니다. Replay contract는 system prompt와 ordered tool name, description, schema version, closed schema를 보존하고 replay item은 message, function call, function result의 정확한 순서와 관계를 보존합니다. Contract는 1 MiB, delta는 16 MiB, Anchor가 선택한 prefix는 64 MiB 및 4096 item으로 제한합니다. Final assistant answer를 수락하기 전에 replay-prefix 또는 model-context capacity exhaustion을 발견하면 delta, outcome, Anchor가 없는 typed failed non-resumable Turn이 됩니다. 완전한 final assistant answer와 필요한 semantic·provider-private item이 모두 각자의 검증과 한도를 통과한 뒤, retained prefix에 적용하는 단계에서 누적 replay capacity만 넘은 경우에만 continuation record가 없는 completed non-resumable Turn을 보존할 수 있습니다. 해당 binding의 이후 Turn은 independently approved compaction 또는 새 binding 전까지 explicit context exhaustion으로 실패합니다. Silent truncation이나 implicit compaction은 허용하지 않습니다. Persisted failed outcome은 required nullable code와 message를 모두 가지며 tool validation은 stable yo.tool.validation.*/v1 code를 사용합니다. Argument와 output은 Activity, 후속 model input, replay persistence 전에 semantic redaction admission을 통과해야 합니다. 이 교체 전 development shape는 같은 schema tag라도 fail closed합니다.


이번 세 번째 reviewed pre-release revision은 바로 앞 replay-delta development shape를 교체합니다. backend_binding_opened는 continuation_strategy를 명시하며 exact_replay는 local_client 또는 managed_server executor를 갖고 backend_managed_state는 executor를 금지합니다. 이는 새 epoch의 seed 방법을 뜻하는 transition.mode와 별개입니다. exact replay binding에서만 model_replay_delta와 outcome의 replay_delta_sequence가 필수이며 delta가 outcome 바로 앞에 있어야 합니다. backend-managed binding에서는 둘 다 금지되고 TurnFinished(completed), payload-free outcome, Anchor가 같은 commit에 연속해서 기록됩니다. 두 exact replay executor의 replay contract, bounds, digest, ordering, Anchor validation은 동일하며 request를 조립하는 위치만 다릅니다. managed_server는 reviewed remote repository 구현 전에는 현재 implementation이 기록할 수 없는 예약 값입니다. 같은 `/v1` tag를 사용한 직전 development shape는 fail closed합니다.

닫힌 `continuation_strategy`는 정확히 두 형태입니다. Exact-replay object는 필수 `mode: exact_replay`, 필수 `executor: local_client | managed_server`, 아래의 optional `replay_profile`을 가지며, 다른 형태는 정확히 `{ mode: backend_managed_state }`입니다. `backend_managed_state`는 `executor`와 `replay_profile`을 금지합니다. Extended exact-replay form의 non-null `replay_profile`은 생략할 수 있고, 생략은 정확한 이전 표현으로서 `semantic-only/v1`로 normalize하며 current producer도 그 값이면 field를 생략합니다. Field가 있으면 최초에는 정확한 `kimi-private-local-plaintext/v1`만 유효하고 이는 semantic item schema `kimi.assistant-message/v1alpha1`을 선언하며 current producer는 이 profile에 field를 반드시 기록합니다. Unknown, null, empty, 다른 값은 fail closed합니다. Normalized 값은 versioned `binding_identity` 비교와 epoch evidence의 일부입니다. Binding-open record를 commit하기 전에 선택된 adapter는 이 값이 complete effective binding의 resolved replay profile과 같음을 증명해야 합니다. Shared semantic validator는 closed field와 cross-record use를 검사하지만 ModelId, Connector, opaque binding value에서 이를 파생하지 않습니다.

닫힌 provider-private replay variant는 정확한 `kind: provider_private_assistant`, 정확한 non-null `schema: kimi.assistant-message/v1alpha1`, 양의 `binding_epoch`, `message`만 가집니다. 다른 item field는 허용하지 않습니다. `binding_epoch`은 containing replay delta epoch과 같아야 하고 open binding은 정확한 replay profile `kimi-private-local-plaintext/v1`을 가져야 합니다. 닫힌 message object는 정확히 필수 `role: assistant`, 필수 UTF-8 string `reasoning_content`, string 또는 null인 필수 `content`, optional `tool_calls`를 가집니다. Reasoning absent/null, content field absent, unknown field는 fail closed합니다. `tool_calls`가 있으면 1~1,024개의 ordered array입니다. 각 item은 정확히 1~4,096 UTF-8 byte `id`, `type: function`, `function`을 가지며, function object는 3~64 ASCII byte이고 `^[a-zA-Z_][a-zA-Z0-9-_]{2,63}$`인 `name`과 최대 4,194,304 UTF-8 byte이며 JSON value 하나로 parse되는 `arguments`만 가집니다. Assistant group 안 ID는 고유하고 generic function-call counterpart와 같아야 합니다. Null, duplicate, malformed, unknown field는 fail closed합니다.

Private item 하나는 matching generic assistant message와 그 뒤의 contiguous function-call item들 바로 다음, function result나 이후 message보다 앞에 있어야 합니다. Content string은 generic assistant content에 byte-for-byte projection되고, null은 빈 generic content로 projection되며 visible content fragment가 없을 때만 유효합니다. Tool call은 generic function-call item과 같은 순서와 field로 projection되고, 없을 때만 생략할 수 있습니다. Generic assistant refusal은 없어야 합니다. Mismatch, 같은 assistant group의 두 번째 private item, 짝이 없는 private item, 다른 replay profile 아래의 private item은 complete delta를 실패시킵니다. Exact Kimi private-replay Connector의 wire projection에서만 private message가 generic assistant group을 대체하므로 assistant object 하나만 전송됩니다. Generic item은 frontend-neutral visible replay authority로 남습니다.

Source Anchor가 선택한 replay prefix에 provider-private item이 있으면 replacement `transition.mode: exact_replay`는 target이 같은 complete binding identity와 replay profile을 기록하거나 독립적으로 검토된 lossless-conversion schema가 있을 때만 유효합니다. Converter가 없으면 다른 모든 target은 `lossy_handoff`를 사용해야 하며, target이 `semantic-only/v1`이어도 item을 버리면서 exact replay를 기록하면 semantic admission이 실패합니다.

Provider-private item 하나의 complete canonical JSON encoding은 16 MiB로 제한되며, 같은 16 MiB delta와 64 MiB prefix ceiling 안에서 한 번만 계산되고 추가 용량이 되지 않습니다. Reasoning, content, ID, name, argument fragment는 초과 byte를 보존하기 전에 점진적으로 검사하고 최종 JSON escape도 canonical delta metric에 포함합니다. Snapshot은 item의 정확한 값, 상대 순서, replay profile, epoch을 보존하고 같은 projection과 bound를 다시 검증합니다. Private admission 실패는 item만 빼거나 redact하지 않고 delta 전체를 거절하므로 outcome과 Anchor도 만들어지지 않습니다.


이번 네 번째 reviewed pre-release revision은 바로 앞 continuation-strategy-aware anchored-session shape와 같은 format generation을 additive extension합니다. Replay message는 정확한 role과 visible UTF-8 content 외에 독립적인 optional visible refusal을 가질 수 있습니다. Refusal은 assistant message에만 유효합니다. Field가 없으면 refusal이 없다는 뜻이고, 존재하면 빈 문자열 `""`도 포함하는 non-null UTF-8 JSON string이어야 하며 null과 다른 타입은 fail closed합니다. Content와 refusal의 decoded UTF-8 bytes를 각각 정확히 보존하고 각자 16 MiB로 제한합니다. 기존 16 MiB delta와 64 MiB 및 4096 item replay prefix 제한은 JSON escape 뒤의 전체 canonical encoded delta bytes에 적용됩니다. System·developer·user message의 refusal은 공통 evidence validation과 wire decoding에서 fail closed해야 합니다.

Refusal이 없는 직전 revision의 모든 유효 record는 확장된 현재 shape에서도 current-generation record입니다. 따라서 refusal이 없는 message와 있는 message가 섞이는 것은 top-level format generation 혼합이 아니며, 서로 다른 `format` discriminator가 섞이는 경우만 계속 fail closed합니다. 새 reader는 직전 record를 그대로 읽지만, 직전 closed-shape reader는 refusal이 실제로 기록된 새 record를 unknown field로 거부합니다. 따라서 기존 Session은 refusal-bearing replay delta가 처음 저장되기 전까지만 이전 binary로 downgrade하여 읽을 수 있고, 그 이후에는 해당 Session이 fail closed합니다. 이 revision은 migration, dual write, downgrade shim 없이 이 비대칭적인 공개 전 data impact를 명시적으로 수용합니다. `format: anchored-session`, checksummed physical envelope, 다른 semantic record는 바뀌지 않습니다.

이번 다섯 번째로 명시적으로 검토된 공개 전 semantic `/v1` 변경은 같은 `format: anchored-session` generation의 additive extension입니다. 물리 `yo.session-record/v1` schema, top-level field, record-kind grammar, discovery object, `crc32c/v1` 표현, checksum domain·preimage는 byte-for-byte 그대로이고 이미 bind된 payload string이 위 private item과 replay-profile evidence를 포함할 수 있게 됩니다. 이전의 모든 유효 semantic record는 계속 유효하고 current reader는 이전 delta와 이후 private-bearing delta가 섞인 Session log를 다시 쓰지 않고 받아들입니다. 이전 semantic reader는 새 item variant나 replay-profile field를 unknown으로 거절하므로 둘 중 하나가 저장되기 전까지만 downgrade-readable합니다. 이 공개 전 비대칭 영향에는 migration, dual write, item omission, downgrade shim이 없습니다. Exact fixture는 이전 byte 불변, current mixed-history recovery와 snapshot, 두 새 shape에 대한 preceding-reader failure, canonical bound 계산, 확장 payload의 CRC coverage, 위에 정의한 null·omission·unknown-field·order·projection·schema·profile·epoch·duplicate case의 거절을 증명해야 합니다.

제한된 `context_compaction_handoff` record는 양수인 source/successor binding epoch, source Continuation Anchor와 commit된 boundary, 정확한 versioned context-strategy identity, 양수 `input_token_limit`, 정확한 compaction 전 및 rebuild 후 input-token count, 정확한 visible UTF-8 summary, 처음 retained semantic sequence, provider-private state가 삭제되었는지를 식별해야 합니다. Private state가 삭제되었다면 record에는 제한된 schema identity, presence, encoded byte count, loss classification만 포함하며 hidden byte는 포함하면 안 됩니다. Strategy identity는 기존의 제한된 versioned-profile grammar를 사용하고 summary는 새 output-size policy 대신 기존 per-message decoded UTF-8 및 canonical replay-prefix limit을 사용합니다. Sole semantic writer는 successor epoch에서 request를 수락하기 전에 `backend_binding_closed(reason: replaced)`, handoff, `backend_binding_opened(reason: lossy_handoff)`를 그 순서로 atomically append해야 합니다. Failure, 이 commit 전 cancellation, rebuild된 strategy가 `Admit`이 아닌 경우에는 이 transition record를 하나도 append하지 않으며 모든 original record를 authoritative하고 byte-unchanged 상태로 둡니다. Recovery와 snapshot은 완전한 source-Anchor, boundary, retained-sequence, epoch graph를 검증하고 summary 처리된 prefix를 몰래 재사용하지 않고 정확한 summary 뒤에 retained semantic suffix를 이어 successor model context를 복원해야 합니다. 이는 같은 `format: anchored-session` semantic generation의 additive pre-release extension입니다. Physical envelope, discovery object, checksum 표현과 preimage는 바뀌지 않고 current reader는 prior record를 수락하지만 prior closed-shape reader는 새 record를 거절하며 migration, dual write, omission, compatibility shim은 제공하지 않습니다.

## 이유

첫 릴리스 전 `/v1`을 다시 닫으면 실험 번호를 공개 호환성 부담으로 남기지 않습니다.
Binding, accepted request, resumable outcome을 별도 record로 보존하면 Codex처럼 요청과
결과 ID가 같은 backend와 Kimi처럼 다를 수 있는 backend를 같은 의미 계약으로 다룰 수
있습니다. Anchor는 문자열을 복사하지 않고 앞 record의 JournalSequence를 참조하므로
작고 검증 가능하며, 기존 물리 envelope checksum만으로 새 의미 payload까지 보호합니다.
Content와 refusal을 분리하면 Chat Completions의 visible field를 정확히 replay할 수 있고,
assistant-only 제한은 다른 API dialect가 user·developer·system message에 refusal 의미를
임의로 부여하는 일을 막습니다.
