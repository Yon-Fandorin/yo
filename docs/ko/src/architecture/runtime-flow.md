# 실행 흐름

변경이 크레이트 경계를 지나거나 오류 메시지만으로 소유자를 알기 어려울
때 이 흐름을 사용한다. 여기에는 현재 구현 경로가 담겨 있다. 각 경계가
어떤 의미여야 하는지는 계속 Methexis가 기준이다.

## Prospective activation 검수

이후 독립 activation 하나는 trusted 상태가 되기 전에 검수할 수 있다.

```text
깨끗한 candidate worktree 안의 정확한 activation request
  ↓ trusted v1alpha3 capability + 정확한 4개 경로 activation-only diff 요구
  ↓ trusted develop 기준 + predecessor CAS + 승인된 Checkpoint 검증
검수 전용 prospective ContextBuild
  ↓ activation 뒤 같은 exact Checkpoint가 재사용하는 결정론적 BuildId
v1alpha3 review packet
  ↓ request + 제안 Checkpoint + 제안 active record + 완전한 diff 결속
prospective 검수 증거만 생성; activation이나 일반 eligibility는 부여하지 않음
```

일반 후보는 계속 active-authority packet 경로를 쓴다. prospective 경로는 proposal을
추론하거나 active authority로 fallback하지 않으며 자기 구현이나 workflow를 바꾸는
후보를 검수하지 않는다.

## Model-service와 OpenAI-compatible connector

provider 중립 service 입력, 명시적인 remote API dialect, Yo-managed loop는 하나의
typed 경로를 이룬다.

```text
설정된 ProviderId + AccountId + 정규화한 endpoint
  ↓ optional base profile + whole-field model override
ModelId 하나의 완전한 EffectiveModelProfile
  ↓ 정확한 ModelCatalog namespace lookup
EffectiveModelBinding
  ├── 명시적인 ApiDialect → 정확히 하나의 built-in ConnectorId
  └── 정규화한 HTTPS base endpoint
ModelContextProfile
  ↓ optional known hard output cap + 주입한 tokenizer profile
NativeModelBackend가 retained replay + 현재 Turn delta를 검사
  ↓ 이번 round의 exact final connector payload를 다시 만들고 계산
known hard max 이하의 양수 request-local cap 또는 unknown cap 생략
  ↓ known: exact input + cap <= input limit; unknown: exact input < input limit
admission된 connector dispatch

선택한 config 경로와 같은 디렉터리의 credentials.yaml
  ↓ no-follow handle 하나, regular file, 현재 owner, 0600에 해당하는 권한,
    제한된 크기, 안정적인 metadata
변경 불가능한 CredentialStore
  ↓ 정확한 ProviderId + AccountId lookup
원문을 감춘 ApiCredential
  ↓ yo-cli의 exact identity+dialect 조립
외부 OpenAiResponsesConnector, OpenAiChatCompletionsConnector 또는 KimiChatCompletionsConnector
POST <정규화한 base>/responses
  또는 <정규화한 base>/chat/completions
  ↓ bearer auth + 같은 origin의 bounded redirect + finite deadline
dialect별 bounded text/event-stream decoder
  ├── correlation을 보존한 text, visible refusal, optional reasoning delta
  ├── 정확한 function call identity, 이름, argument byte
  └── completed, incomplete 또는 failed terminal + usage
  ↓ NativeModelBackend
semantic AgentMessage, ModelWork, ToolCall Activity
  ↓ 고정된 ToolRegistry schema 검증, host semantic-admission gate,
    정확한 approval binding
주입된 ToolExecutionHost의 직렬 단일 실행 시도
  ↓ 제한한 output이 같은 semantic-admission 경계를 통과
다음 remote request 전에 durable한 ToolResult Activity
다음 model round 또는 재개 가능한 semantic replay delta
```

`yo-core::model_service`가 이 resolution과 validation을 소유한다. credential
파일이 없으면 아무것도 만들지 않고 빈 snapshot을 반환하며, 기존 파일이
안전하지 않거나 형식이 잘못되면 실패-폐쇄한다. API key에는 environment
fallback이 없고 diagnostic formatting은 내용을 노출하지 않는다. 표시 이름은
optional metadata일 뿐 identity나 routing에 참여하지 않는다.
`NativeModelBackend`는 의도적으로 빈 message item도 포함해 assistant output item
하나의 모든 visible content part를 하나의 `AgentMessage` Activity로 투영한다.
reasoning은 `ModelWork`로 남으므로 print mode는 reasoning을 노출하거나 답으로
오인하지 않고 마지막으로 완료된 답을 선택할 수 있다.
`yo-core::model_connector`는 중립 port를 소유하며 `api_dialect`에서 built-in connector
identity 하나를 파생한다. `yo-cli`는 Provider probing이나 fallback 없이 그 exact identity와
dialect를 독립 Responses, Chat Completions 또는 Kimi crate에 연결한다. Responses는 `responses`
segment를 정확히 하나 붙이고 Chat Completions와 Kimi는 정확히 `chat/completions`를 붙인다. 어느
경로도 `v1`을 하나 더 붙이거나 provider conversation authority 또는 built-in tool을 켜지
않는다. Chat decoder는 index 0인
choice 하나, finish 뒤 final usage와 `[DONE]` 순서를 요구하고 content와 refusal을 독립적으로
보존하며 index가 있는 tool-call fragment를 correlate한다. tool call의 첫 fragment는 비어
있지 않은 ID와 function name을 하나 고정한다. 후속 fragment는 이를 생략할 수 있고,
호환 API가 반복 ID를 명시적인 빈 문자열로 보내면 omission으로 정규화하지만 다른 비어
있지 않은 ID나 function name은 여전히 stream을 실패시킨다. 두 dialect 모두 SSE event와 누적
payload를 읽는 동안 제한하며 cancellation은
header·stream·queue wait를 중단한다. 기본 agent 경로에는 absolute model-request deadline이
없다. 그래도 connection setup은 30초, 각 redirect attempt의 response header는 5분,
successful-stream inactivity는 5분, non-success error-body inactivity는 30초, 각 내부 event
handoff는 5분으로 제한한다.
비어 있지 않은 raw HTTP body chunk만 SSE decode 또는 error-body retention 전에 body
inactivity clock을 reset하며, observation마다 새로운 event-handoff wait를 시작한다.
`yo-backend-managed`가 `yo-core::AgentBackend` 뒤의 semantic Activity와 제한된
model/tool loop를 소유한다. 매 dispatch 전에 catalog가 선택한
tokenizer profile로 실제 request를 계산하기 전에 retained replay prefix와 현재 Turn delta를
함께 검사한다. known hard output maximum이면 그 이하의 양수 request-local cap을 유한하고
엄격히 감소하는 방식으로 선택하며 candidate payload마다 다시 만들고 다시 계산한다. maximum이
unknown이면 connector payload에서 cap을 생략하고 exact input count가 input limit보다 작아야 한다.
Final assistant semantic/private delta를 적용하기 전의 capacity failure는
`code=context_exhausted`인 Failed Turn을 기록하고 continuation anchor를 만들지 않으며, 한도를
넘는 remote request를 보내지 않고 이후 Turn이 같은 binding을 쓰지 못하게 latch한다. 이미
유효한 final assistant delta를 적용하는 동안 발생한 capacity exhaustion만 현재 Turn을 resumable
evidence나 anchor 없이 완료한다.
원시 tool
argument는 schema 검증을 거치고 tool output은 제한된 뒤, 주입한 host gate가 Activity,
replay, 이후 request에 들어갈 수 있는 semantic 형태를 결정한다. backend는 이렇게
admission된 call/result replay만 기록하고, 승인과 실행 시도 Activity를 Journal에 남길
수 있을 때까지 승인된 effect를 미루며, 각 terminal response의 usage를 정확한
Provider·Account·Model·connector·endpoint·완전한 resolved profile에 귀속한다. process host가 startup
선택, 이 입력들의 조립, 구체적인 local tool을 소유한다.

새 local-client exact-replay Session은 첫 model request 전에 닫힌 context policy를
commit한다. 기본값은 정확한 전체 Connector tokenization payload의 85%에서 pressure를
알리고 90%에서 portable-summary 압축을 선택하며, 보고된 cache-read token은 이 점유량에서
빼지 않는다. 지원하는 경계에서 managed backend는 이미 선택한 binding을 통해 tools-disabled
summary request를 정확히 한 번 보내고, summary 입력에서 provider-private replay를 제외하며,
고정된 `Context Checkpoint` Markdown 형태를 검증하고, 가장 최신 complete replay group과
현재 input을 보존한 뒤 successor payload 전체를 다시 계산한다. Summary 중 나타나는
provider-private event는 일시적인 값으로 버려 checkpoint에 넣지 않고, optional reasoning
usage를 보고하지 않은 connector는 `null`을 유지하되 input·output·total usage는 필수다.
Core는 proposal을 정확한
Journal group에 연결해 checkpoint를 원자적으로 commit한 뒤에만 backend가 replay를 바꾸거나
successor request를 dispatch하게 한다. 내부 successor request마다 writer가 Session, Turn,
정확한 outbound exchange sequence에서 도출한 별도의 UUIDv4 operation root를 부여한다.
Recovery는 이전 accepted request가 있는 active Turn이거나 active checkpoint가 submitted
input을 가로지른 뒤 첫 request인 경우에만 이 identity를 허용하며, 같은 byte로 완료된 Turn에
작업을 붙일 수 없다. Checkpoint는 Session-global context epoch만 전진시키며,
resume은 policy, epoch, replay root, semantic group 경계를 함께 복원한다.

Live와 archived Transcript는 commit된 policy와 redacted checkpoint observation을 투영한다.
Chat은 summary body나 raw artifact를 노출하지 않고 before/after 측정값, limit, context-epoch
전이, source boundary, retained-group budget, artifact receipt 수, 공개된 loss class를 보여준다.
요약된 source group 하나에서 비어 있지 않은 tool-output byte가 같으면 복구 불가능한 중복
receipt를 만들지 않고 disclosure receipt identity 하나를 공유한다.

Automatic 실행은 새 Turn의 첫 request 경계와 완전히 승인된 tool call/result suffix 뒤의
경계를 모두 지원한다. 후자에서 managed backend는 관련 Activity가 모두 끝난 뒤 정확하고
단조 증가하는 active suffix를 먼저 공개하고, core는 checkpoint proposal을 받기 전에 이를
현재 input command와 마지막 durable `ActivityFinished`에 결속한다. Pending approval·tool
effect·model stream, 조작된 suffix, complete boundary가 없는 ordinary accepted request는
summary가 durable해지기 전에 fail-closed한다. Idle `/compact [GUIDANCE]`는 automatic
threshold 아래에서도 같은 단일-request pipeline을 사용하고 checkpoint 경계가 끝날 때까지
새 Turn을 받지 않는다. Admission은 worker가 command를 보기 전에 이 구간을 예약하므로 뒤따른
prompt와 binding replacement가 checkpoint를 앞지를 수 없고, 보존된 prompt는 activation 뒤
change signal로 다시 시도된다. Manual compaction을 허용하지 않는 policy나 완료된 history가
너무 적은 Session은 typed nonterminal control outcome을 반환하며 같은 Session은 이후 prompt에
계속 사용할 수 있다. 잘못된 guidance와 queued 또는 active Turn과의 admission 경합도 worker를
종료하지 않고 같은 control outcome을 사용한다. Pressure observation은 typed durable telemetry로 유지하되 Chat은
JSON을 model work처럼 노출하지 않고 간결한 사람이 읽을 수 있는 상태로 투영한다. Disabled 또는
exact-replay-only policy도 압축 대신 거부하며 malformed output, 불완전한 usage 귀속, 줄지
않은 결과, trigger에 계속 걸리는 successor payload에는 retry나 fallback이 없다.

새 local-tools Session의 startup은 `list_files`, `read_files`, `edit_file`,
`write_file`, `run_command` 순서인 5개 basic registry를 고정한다. Resume은 durable
replay Projection을 exact basic manifest, 직전 3개 legacy manifest, empty manifest와
비교하고 unknown 또는 mixed Projection은 기존 read-only 실패 경로로 보낸다. 이후 model
binding replacement도 이미 선택한 registry revision을 전달하므로 Session의 tool history를
조용히 upgrade하지 않는다.

File host는 execution attempt 전에 semantic-admission 경로에서 구체적인 item·number·path·
content bound를 검증하고 path를 열기 전에 방어적으로 다시 parse한다. `read_files`는 유지한
workspace directory descriptor 아래의 일반 UTF-8 file을 각각 독립적으로 capture하고,
순서가 보존된 compact-JSON window 또는 item별 bounded error를 반환한다. `edit_file`은
capture한 original 하나에서 unique하고 겹치지 않는 match를 모두 계산하며 `write_file`은
complete file image 하나를 제공한다. 두 mutation tool은 host instance 하나 안에서
직렬화하고 같은 parent에 owner-only scratch file을 쓴 뒤 유지한 identity를 검증하여 rename
한 번으로 공개한다. 실패하면 여전히 소유한 scratch state만 닫고 제거한다. 다른 process와
editor는 이 in-memory lock 참여자가 아니라 명시적인 last-publisher-wins actor로 남는다.

Local `run_command` host는 비어 있지 않은 stdout 또는 stderr chunk를 하나의 공유 progress
signal로 취급한다. 기본 attempt에는 5분 output-inactivity window가 있고 absolute execution
deadline은 없다. Runtime policy는 execution request에 absolute deadline 하나를 추가할 수
있으며, 이 clock은 한 번 시작되고 output으로 reset되지 않는다. Inactivity, 선택적인
absolute deadline, cancellation은 모두 유한한 process-group termination, child reap, output
drain 경로로 들어가며 diagnostic은 그 원인들과 cleanup failure를 구분한다. Host는 command를
자동으로 재시도하지 않는다. 전용 waiter가 spawn부터 단 한 번의 최종 wait까지 child를
소유하므로, 제한된 result 경로가 cleanup failure를 보고한 뒤 더 늦게 waitable이 된 child도
버리지 않는다. 서로 독립적인 stdout과 stderr reader는 보존 한도를 넘은 뒤에도 계속
drain하고, 메모리에는 생략 byte 표식과 제한된 head·tail view만 남긴다. 따라서 output
truncation이 `EPIPE`나 command effect 변경을 일으키지 않는다. command process group 밖의
writer가 cleanup grace 뒤에도 pipe를 열어 두면 명시적인 shutdown wake 하나가 두 local read
end를 닫고 reader thread 둘을 join한다. 이 attempt는 thread나 descriptor 소유권을 무기한
남기는 대신 cleanup failure를 보고한다.

열린 모든 backend binding은 continuation strategy를 선언한다. 현재 Yo-managed
경로는 local client의 exact replay를 선언하고 Codex와 Grok은 backend-managed state를
선언한다. Exact replay는 별도의 제한된 `ModelReplayDelta`, 그 delta를 가리키는
payload-free resumable outcome, Continuation Anchor 순서로 commit한다.
Backend-managed state는 replay-delta 참조가 없는 outcome을 Anchor보다 먼저
commit한다. Recovery는 backend 이름으로 소유권을 추론하지 않고 이 순서와
strategy별 존재 조건을 검증한다. Managed-server exact-replay executor는 예약된
contract 값이며 현재 이를 선택하는 backend는 없다.

### 모델 선택과 교체

startup은 optional `--model TARGET_REFERENCE` 하나를 받는다. 정확한 `host:codex`와
`host:grok`은 각각의 local delegated HostTarget을 선택한다. ModelTarget 표기는 `Model`, `Provider::Model`,
`Provider:Account:Model`이다. Provider와 Account는 `%`를 `%25`로, `:`를 `%3A`로
encode하고 vendor가 소유하는 Model suffix는 바꾸지 않는다. separator 우선순위로
파싱하지 않고 설정된 완전한 좌표에서 가능한 표기를 만들어 대조하므로 vendor
ModelId에는 `:`, `/`, `.`이 들어갈 수 있다. bare model reference는 현재 Provider와
Account 안에 머물고, namespace가 없으면 catalog 전체에서 정확히 하나인 ModelId여야
한다. qualified 두 형식은 각각 정확한 Provider와 Model에 해당하는 Account가 하나이거나,
정확한 완전 좌표 하나여야 한다. 없거나 모호하면 안정적으로 정렬한 canonical 완전
좌표와 함께 명시적으로 실패한다.

새 Session에서는 명시적인 invocation target, 저장된 `connections.yaml` preference,
policy default 순으로 우선한다. `config.yaml`은 모델 target을 제공하지 않는다. 선택 가능한
모든 계층이 없으면 host를 조용히 선택하지 않고
Session 생성 전에 정확한 `yo connect`, `yo --model host:codex`, `yo --model host:grok` 안내로 실패한다.
`yo default TARGET`은 정확한 HostTarget 또는 설정된 ModelTarget 하나를 admission하고
저장하며, `yo default --unset`은 이 저장 계층만 지운다. 재개하는 Yo-managed Session은
저장 preference를 읽지 않고 최신 durable binding을 bare namespace로 쓰며 startup 기본값은 그
namespace를 바꾸지 않는다. 정확한 `host:codex` 또는 `host:grok`은 일치하는 delegated-host resume을 확인하며, 서로 다른
cross-backend target은 handoff가 아직 미뤄져 있으므로 명시적으로 실패한다.

저장된 각 complete ModelTarget에는 binding identity, catalog availability, credential,
`last_failure`와 분리된 operator activation 상태도 있다. `yo model disable TARGET`은
complete binding을 보존하지만 새 startup 기본값, explicit 또는 bare `--model` 선택,
live picker, live replacement, 다음 Turn 예약에서는 typed 이유 `disabled by operator`로
차단한다. Picker는 그 이유와 함께 행을 계속 보여 준다. 정확한 저장 preference를
disable하면 같은 public CAS에서 지우고, `yo model enable TARGET`은 preference를 다시
만들지 않는다. 두 명령은 모두 idempotent하고 복구된 connection-operation lane을
사용하며 Provider request를 만들지 않고 credential byte를 읽거나 다시 쓰지 않는다.

Disable은 이미 admit된 Turn을 중단하지 않는다. 저장 native Session은 `--model`
override가 없고 durable complete binding이 정확히 그대로일 때만 disabled model을
resume할 수 있다. 같은 좌표를 포함한 explicit override는 새 선택이므로 차단되고,
달라진 complete binding도 replacement 작업으로 차단된다. Enable은 durable disabled
marker를 제거한다. Exact credential rotation과 exact complete group reimport는
activation을 보존하지만 바뀐 complete binding은 enabled 상태로 시작한다.

새 native ModelTarget Session에서 `--no-tools`를 지정하면 선택한 complete model
binding은 바꾸지 않고 local tool registry를 빈 상태로 고정한다. 이후 request는 현재
tool definition과 tool choice를 생략하며, Session을 flag 없이 재개해도 exact replay가
빈 registry를 보존한다. 이 option은 `--resume`, `--continue`, delegated HostTarget과
함께 사용할 수 없고, live native model 교체도 Session의 빈 frozen registry를 유지한다.

편집 가능한 Chat에서 유일한 prompt token의 slash를 입력하고 cursor가 draft 끝에 있으면
prompt에 인접한 command palette가 열린다. 이어지는 문자는 순서가 정해진 `/help`,
`/model`, `/compact`, `/exit` catalog를 filtering한다. `command/`의 각 child는 command 하나의 ID,
invocation, description, typed effect를 소유한다. 얕은 registry는 uniqueness 검증,
순서 합성, 항목 filtering, help Projection만 담당한다. 공용 overlay slot은 위·아래 이동,
Enter 또는 Tab acceptance, Esc 닫기를 소유한다. open됐지만 아직 표시되지 않은 panel은
key를 소유하지 않는다. 한 번 보인 instance가 refresh되면 token과 revision을 포함한
presentation receipt가 일치하는 frame이 commit될 때까지 이동과 acceptance에 fence를
둔다. stale하거나 대체된 frame은 이 fence를 해제할 수 없다. 일치하는 hidden commit은
fence를 해제하는 동시에 instance를 unpresented로 표시하므로, 여전히 key를 소유하지 않는다.

palette가 unknown 또는 아직 표시되지 않은 partial slash draft를 소유한 상태에서 Enter를
누르면 로컬 unknown command를 알리고 draft를 보존한다. 표시된 partial query에서는 선택된
enabled row를 대신 accept할 수 있다. 실제로 보인 palette를 닫은 Esc만 정확히 같은 unchanged
draft의 ordinary submission 한 번을 허용한다. 대기 중인 Activity가 있으면 그 Activity에
답하고, 없으면 Turn을 시작하거나 frontend가 관찰한 정확한 `TurnRef`를 steer한다.
draft를 편집하면 이 예외는 취소된다. `/help`는 로컬 command 요약을 추가하고 `/model`은
Activity 응답 처리보다 먼저 selection flow에 들어가므로 둘 다 대기 중인 Activity를
암묵적으로 답하거나 취소하지 않는다. `/exit`는 명시적인 process-lifecycle 예외이며 기존
runner 종료 경계를 사용한다. 읽기 전용 view에서는 palette가 비활성화되지만 pending
Activity는 이 로컬 command를 숨기지 않는다.

`/compact`는 idle Yo-managed control command다. 선택적인 suffix는 일반 prompt 제출이 아니라
summary request를 위한 bounded user guidance다. Active Turn에서는 draft를 보존하고 idle이
필요하다고 알리며, delegated Codex와 Grok Session은 Yo-managed checkpoint를 지원한다고
가장하지 않고 unsupported로 거부한다. 이 정상적인 거부는 nonterminal control result이며
delegated Session을 닫지 않는다.

Turn이 보이는 동안 제출한 일반 prompt는 정확히 그 `TurnRef`를 `yo-core`까지 전달한다.
worker가 이미 해당 Turn을 끝냈다면 core는 같은 text를 새 Turn으로 재해석하지 않고 steer를
거절한다. backpressure와 retry도 같은 immutable intent를 보존한다.

Yo-managed TUI에서 `/model`은 Provider, Account, Model 순서로 정렬한 항목을 범용
selection panel에 연다. label에는 optional display name을 쓰지만 각 행의 identity는
완전한 안정 좌표다. `/model MODEL_REFERENCE`는 startup과 같은 resolver를 사용하므로 bare
형식은 현재 namespace에 머물고 qualified 형식은 설정된 다른 Provider나 Account를 선택할
수 있다. delegated host로 시작한 live Session은 이 picker를 노출하지 않는다. idle에서의
선택은 즉시 host 교체를 요청한다. active Turn, pending Activity 또는 pending prompt
admission 중에는 다음 Turn을 위한 model 하나를 대신 예약하고 사용자에게 알린다. 현재 Turn,
steer, Activity 응답은 이전 model을 계속 사용한다. 이후 선택은 예약을 교체하며 현재 model을
고르면 예약을 취소한다.

정확한 active Turn의 durable 완료만 예약을 확정한다. memory-only 완료나 durability gap은
이전 model을 유지하면서 예약을 지우고 실패를 화면에 알린다. 예약이 확정되면 TUI가 terminal
input loop를 동기적으로 벗어나고, terminal input이 중단된 동안 process host가 교체를 수행한
뒤 보존된 TUI가 다시 진입한다.

frontend 중립 `ModelSelectionController`가 이 resolution 규칙을 소유한다. 선택을
accept하면 process host는 현재 binding을 유지한 채 startup credential snapshot,
tokenizer, connector, tool registry, tool host로 후보 backend를 구성하고 검증한다.
준비 실패는 보존한 TUI에 알린다. 이어서 Session worker가 exact-replay 전환을 원자적으로
commit한다. durable 실패면 후보를 버리고 기존 backend를 계속 사용하며, 성공하면 이전
binding epoch를 닫고 replacement epoch 하나를 연 뒤 backend를 제자리에서 교체한다.
같은 TUI와 Yo Session이 계속 활성 상태이고, 선택은 설정 기본값을 변경하지 않는다.

## 시작

프로세스 정책과 agent Session이 준비된 뒤에만 터미널을 획득한다.

```text
yo-cli
  표시 mode, glyph profile, optional 모델 좌표 해석, cwd 확보
  새 Session을 위해 config.yaml과 생성하지 않는 connections.yaml snapshot 하나 capture
  invocation > 저장 preference > policy default 순서로 resolve
  저장 모델을 선택하면 해당 Provider/Account의 정확한 credential 읽기
  TerminationCoordinator 설치
  Host identity와 Session repository 열기
  workspace 정규화와 SessionDescriptor 생성
  선택한 Codex/Grok delegated transport 시작 또는 dialect에서 파생된 managed model backend 조립
      ↓
yo-core AgentSession
  worker 시작
  descriptor envelope 시도
  CreateSession
      ├── CodexBackend → app-server initialize + thread/start
      ├── GrokBackend → ACP initialize + cached-token 인증 + session/new
      └── NativeModelBackend → local exact-replay Session state 연결
      ↓
yo-core
  SessionCreated
      ↓
yo-tui
  터미널을 획득하고 Inline 또는 Fullscreen mode 진입
```

| 단계 | 현재 소유자 | 확인할 내용 |
|---|---|---|
| 1 | [`yo-cli/src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs), [`yo-cli/src/connection.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection.rs) | `run`이 표시 옵션·작업 디렉터리·command-local 설정을 확보하고 새 Session의 저장 preference를 상태 생성 없이 읽는다. 종료 coordinator를 설치하고 Host identity와 Session storage를 열며 workspace를 canonicalize한 뒤 시각이 일치하는 UUIDv7 `SessionDescriptor`를 만든다. Resume은 저장 preference를 읽지 않는다. |
| 2 | [`yo-cli/src/model.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/model.rs), [`yo-backend-delegated-codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-codex/src/lib.rs), [`yo-backend-delegated-grok`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-grok/src/lib.rs), [`yo-backend-managed`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/managed/src/lib.rs) | process host가 invocation·저장·operator 계층을 resolve한 다음 선택한 delegated stdio transport를 시작하거나 startup snapshot과 주입된 tool로 managed binding을 조립한다. 모든 경로는 worker가 backend를 소유할 때까지 model 작업을 미룬다. |
| 3 | [`yo-core/agent_session`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/mod.rs) | `AgentSession::start_cancellable_with_repository`가 backend와 local repository를 `yo-agent-runtime`이라는 worker thread로 넘긴다. 종료 관찰을 막지 않으면서 시작 완료를 기다린다. |
| 4 | [`yo-core/agent_session/worker.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/worker.rs) | `AgentWorker::initialize`가 descriptor-only Journal envelope를 먼저 시도한 뒤 `AgentRuntime`을 통해 `CreateSession`을 보낸다. storage pressure가 있으면 descriptor와 이후 activity를 복구 가능한 volatile prefix로 함께 유지한다. |
| 5 | [`yo-backend-delegated-codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-codex/src/lib.rs), [`yo-backend-delegated-grok`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-grok/src/lib.rs), [`yo-backend-managed`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/managed/src/lib.rs) | Codex는 `initialize`와 `thread/start`, Grok은 ACP 초기화·cached-token 인증·`session/new`를 수행한다. managed backend는 provider 요청 없이 local exact-replay state를 연결한다. 각 경로는 semantic engine이 `SessionCreated`를 만들게 한다. |
| 6 | [`yo-tui/runner/unix.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/unix.rs) | `run_session_with_mode`가 첫 터미널 소유 세대의 input과 터미널 상태를 획득하고 이미 선택된 표시 mode로 들어간다. |

handshake 중에 종료 요청이 오면 `AgentSession::start_inner`가 취소
callback을 관찰하고 backend 중지를 요청한 뒤 worker 정리를 기다린다.
그리고 TUI에 Session을 넘기지 않은 채 반환한다. 이 경우 터미널 mode
코드가 아니라 여기서 조사를 시작한다.

공개 host flag는 표시를 위한 `--inline` 또는 `--fullscreen`과 built-in
ASCII glyph profile을 위한 `--ascii`이며 순서와 관계없이 사용할 수 있다.
표시 flag를 생략하면 Inline, `--ascii`를 생략하면 호환 기본값인 Rich를
사용한다. 알 수 없는 flag, 반복한 `--ascii`, 둘 이상의 표시 flag는
provider나 터미널을 시작하기 전에 실패한다. 선택한 glyph profile로 보존할
`TuiSession`을 생성하므로 준비한 frame과 마지막 plain session output은
같은 committed appearance snapshot을 읽는다. Glyph 선택은 명시적이며
`TERM`이나 `NO_COLOR`를 검사하지 않는다. 별도로 CLI는 명시적인
`COLORTERM=truecolor|24bit`를 TrueColor, `256color`가 든 `TERM`을 Limited,
색상이 억제되었거나 증거가 없으면 Unknown으로 분류하며 TrueColor에서만 RGB
activity ramp를 사용한다. CLI는 자신이 아는 backend 이름과
홈 경로를 줄여 쓴 작업 디렉터리 label도 보존되는 session에 전달한다. 이
label은 화면 표시용 metadata일 뿐 backend Session을 선택하거나 식별하지
않는다.

계약:
[session publication](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.session-publication.md),
[frame 일관성](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.frame-consistency.md),
[glyph profile](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.glyph-profiles.md),
그리고
[resolved cell style](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.resolved-style.md).

## One-shot print frontend

`yo -p "prompt"`와 `yo --print "prompt"`는 `TuiSession`을 만들거나 터미널
상태를 얻지 않고 일반 새 Session 시작 또는 정확한 `--resume SESSION_ID` 복구를
사용한다. 위치 prompt, TTY가 아닌 stdin 또는 둘 모두를 Backend 시작이나 Session
복구 전에 비어 있지 않은 UTF-8 입력 하나로 만든다. 둘 다 있으면 stdin이 먼저
오며, stdin 끝에 LF가 없을 때만 host가 LF 하나를 넣는다. TTY stdin은 암묵적인
입력을 제공하지 않는다.

```text
위치 prompt + 선택적인 piped stdin
    ↓ UTF-8 검증과 결정론적 조합
AgentSession까지 일반 새 Session 시작 또는 정확한 저장 Session 복구
    ↓ 변경 불가능한 InputSubmission 정확히 하나
AgentRuntime과 선택한 Backend
    ↕ 일반적인 내부 provider·tool round
TranscriptReader
    ↓ ModelWork·tool Activity·usage·trace·중간 message 무시
완료된 Turn에서 마지막으로 완료된 AgentMessage
    ↓ Session·Backend·process 정리
답변 끝에 LF가 없을 때 LF 하나를 붙인 stdout
```

[`yo-cli/src/print.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/print.rs)는
입력 조합, Submission admission, Transcript 투영, 출력 framing만 소유한다.
backpressure가 걸린 command는 같은 Submission identity로 다시 시도하고, 일치하는
admission outcome과 terminal Turn outcome을 모두 기다린다. 선택한 Backend는 계속
provider 요청과 내부 tool round를 소유한다. `--model`은 startup 선택을 바꾸고
`--no-tools`는 일반 empty tool registry를 선택한다. print mode 자체는 tool 권한이나
승인을 부여하지 않는다. 대신 `--resume SESSION_ID`는 저장된 Session identity,
Provider/Account/Model binding, tool registry, replay 상태, usage, cache, request
lineage를 보존한다. 최종 응답 투영은 복구된 Transcript observation head에서
시작한다. 이 연속 좌표는 복구 중 durable Journal record가 압축되어도 올바르게
유지되므로 이전 Turn의 출력은 제외된다.

승인 요청, 사용자 입력 요청, 거절된 Submission, 중단되거나 실패한 Turn, 완료된
Agent message 누락, 잘못된 입력, startup 실패 또는 정리 실패는 0이 아닌 코드로
종료하며 stdout을 비워 둔다. 진단은 stderr를 사용한다. 성공 응답은 정리가 성공할
때까지 buffering하므로 터미널 제어, 진행 Activity, request trace, Session identity,
usage, cache metric이 답변과 함께 stdout에 섞이지 않는다. print resume은
`--continue`, `--model`, 새 Session 전용 `--no-tools` 제한과 terminal 표시 flag를
거부한다. 복구나 binding 실패 시 새 Session을 만들거나 retry, steer, fallback 또는
다른 Provider 선택을 하지 않는다. 또한 같은 호출에 top-level 하위 명령 token이
있으면 이를 positional prompt로 조용히 해석하지 않고 거부한다. prompt가 하위 명령
이름과 같다면 `--`로 literal prompt의 시작을 명시한다. process 계층은 generation과
정리가 성공한 뒤 이미 framing된 답변을 변경하지 않고 정확히 한 번만 게시한다.

계약:
[첫 coding loop](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.delivery.first-coding-loop.md).

## Workspace reference 도움

Chat에서 유효한 `@query`를 입력하면 agent command와 분리된 다음
nonblocking 경로를 따른다.

```text
PromptEditor + cursor
  ↓ revision에 묶인 trigger snapshot
yo-core local 실행 workspace provider
  ↓ Git ignore를 따르는 파일·디렉터리 + 결정적인 Unicode 정규화 순위
TuiState prompt overlay
  ↓ Tab 또는 Enter
정확한 @query span을 바꾸고 typed identity 보존
```

`yo-tui`는 scan, stale 결과 거절, overlay 입력, editor span 변환을
소유한다. `yo-core::LocalWorkspaceReferenceProvider`가 local 실행 탐색
의미와 background Git·filesystem 작업을 소유하고, `yo-cli`는 이 capability를
생성해 연결만 한다.
candidate와 request/update type은 `yo-core`에 있으므로 remote 실행
provider를 연결해도 filesystem 권한이 frontend로 이동하지 않는다.
inventory는 보이는 파일과 디렉터리를 포함하고 nested Git ignore,
repository exclude, 설정된 global exclude를 따르며 directory symlink를
따라가지 않는다. 각 행은 basename과 dimmed 부모 경로를 왼쪽 읽기 흐름에
함께 두고, 오른쪽 끝은 중립적인 `File` 또는 `Dir` 종류에만 사용한다.
디렉터리 label과 선택 후 token은 `/`로 끝나 입력 중에도 종류가 눈에 보인다.
첫 query는 header에 검색 중 상태를 보여줄 수 있지만 연속 입력 중에는 현재 panel을 유지하고
최신 결과가 도착할 때 한 번만 다시 그려 중간 loading frame이 깜빡이지 않게 한다.
panel title은 `Files`이며 header hint는 활성 binding에서 도출해 key만 강조하고
caption은 dim 처리한다. Rich glyph는 이동에 `↑↓`, ASCII는 `Up/Down`을 쓰고,
익숙한 terminal 표기인 `Enter`, `Esc`, `^C`는 문자 그대로 유지한다.

이 Slice는 structured submission admission 직전에서 의도적으로 멈춘다.
항목을 고르면 token은 눈에 보이게 치환되고 typed reference가 남지만,
그 뒤 Enter를 누르면 draft를 보존하고 아직 structured submission이
연결되지 않았다고 알린다. 승인한 identity를 몰래 plain text로 낮추지
않는다.

## 명시적 skill 지원

유효한 `$query`를 입력하면 같은 prompt trigger 생명주기를 재사용하되,
별도의 frontend 중립 skill port에서 metadata를 찾는다.

```text
PromptEditor + cursor
  ↓ revision-bound $ trigger
CodexSkillReferenceProvider worker
  ↓ 현재 cwd에 대한 Codex skills/list descriptor
Skills overlay
  ↔ Left/Right로 cached 행을 All, Workspace, User, System, Admin 중 하나로 filter
  ↓ Tab 또는 Enter
정확한 $query span을 바꾸고 catalog identity와 revision selector 보존
```

catalog worker는 수명이 짧은 Codex app-server 연결을 소유하며 terminal event
loop를 막지 않는다. Codex가 보고한 `repo`, `user`, `system`, `admin` scope만
사용하고 filesystem path에서 provenance를 추측하지 않는다. 같은 이름도
identity가 다르면 별도 행으로 남고, 비활성 skill은 이유와 함께 보이지만
선택할 수 없다. local adapter는 정확한 `SKILL.md` byte를 hash해 entry revision으로
사용한다. revision을 읽을 수 없는 행은 admission이 나중에 검증할 수 없는
selector를 만들지 않도록 비활성화한다. 새 Skills overlay를 열 때는 새
`skills/list` snapshot을 강제로 읽고 catalog generation을 올린다. 연속 입력은
같은 snapshot을 대상으로 최신 query로 합친다. 선택적인 scope filter는 panel 왼쪽 하단에만 둔다. Left와
Right는 이미 받은 후보만 좁히므로 discovery를 다시 실행하거나 prompt를
재배치하지 않는다.

V1은 accept된 명시적 skill을 최대 하나만 보존한다. 선택은 skill 본문을
읽거나 실행하거나 model context에 주입하거나 draft를 제출하지 않는다.
제출 시점 admission이 정확한 항목을 다시 읽고 검증할 수 있을 때까지 Enter는
draft를 보존하고 실패-폐쇄하며, 보이는 `$name`만으로 충분한 권위라고
간주하지 않는다.

## 활성 Turn 하나

제출된 prompt는 다음 경로를 지난다.

```text
terminal input
    ↓
TuiState::handle
    ↓ 변경 불가능한 InputSubmission
TuiAgentConnection
    ↓
AgentSession queue와 bounded command lane
    ↓
AgentWorker
    ↓ 같은 SubmissionId를 수락 또는 거절
    ↓ AgentCommand::StartTurn or SteerTurn
AgentRuntime
    ├── AgentEngine으로 검증
    ├── yo-core AgentBackend를 통해 수락
    │       ↕ yo-backend BackendAdapter + 중립 evidence
    ├── AgentEngine으로 commit
    └── command와 event를 SessionJournal에 추가
          ↓
Codex app-server adapter
    ↓ BackendEvent
AgentRuntime
    ↓ commit한 뒤 SessionJournal에 추가
AgentSession의 합칠 수 있는 change lane
    ↓ 내용 없는 깨우기 알림
TuiAgentConnection + TranscriptReader + RequestTraceReader
    ↓ 순서가 보장된 AgentPoll::Record / RequestTrace
    ↓
TuiState::observe_record
    ├── 간결한 Chat Projection
    └── chronological Transcript / full-Session Request Projection
          ↓ 선택된 view
completed Surface
    ↓
Inline 또는 Fullscreen presenter
```

`yo-backend`는 Yo command·event, Session·Journal 좌표, host wire type을 가져오지 않고
transport-neutral lifecycle, bounded evidence, bounded child-process JSONL·mailbox mechanism을 정의한다.
`yo-core::AgentBackend`가 generic port를 `AgentCommand`, `BackendEvent`,
`BackendResumeTarget`으로 고정하므로 semantic validation, durable 좌표,
exact replay-profile·schema 해석은 계속 core가 소유한다.

조사할 때 유용한 지점은 다음과 같다.

1. [`TuiState::handle`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/state.rs)는
   변경 불가능한 `InputSubmission` 하나를 캡처한다. 같은 `SubmissionId`의
   `Accepted` outcome이 올 때까지 plain text를 입력창에 보존한다. 그사이
   사용자가 새 draft를 편집했다면 그 새 text는 지우지 않는다. 거절은 draft를
   보존하며, 중복되거나 오래된 outcome은 아무 영향도 주지 않는다.
2. [`TuiAgentConnection`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/agent/mod.rs)은
   좁은 local adapter다. dispatch, retry, submission outcome을 전달하고, 하나로 합쳐진
   Session 변경 알림을 `TranscriptReader`의 크기가 제한된 suffix 읽기로
   바꿔 순서가 보장된 record를 TUI에 제공한다. Session이나 provider
   의미는 소유하지 않는다.
3. [`agent_session/admission.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/admission.rs)는
   Submit을 `StartTurn` 또는 `SteerTurn`으로 결정한다. `Queued`는 bounded
   worker lane이 command 소유권을 받았다는 뜻일 뿐 최종 수락이 아니다.
   state lock이 사용 중이거나 lane이 가득 찼다면, 같은 `SubmissionId`를
   가진 내부가 드러나지 않는 pending command를 TUI loop가 다시 시도하도록 반환한다.
   첫 dispatch가 그 ID를 Session에 예약하므로 재사용은 다른 backend command가
   실행되기 전에 거절된다.
4. [`AgentWorker`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/worker.rs)만
   runtime을 실행하고 polling할 수 있다. runtime과 backend 수락이 성공한 뒤
   정확한 ID의 `SubmissionOutcome::Accepted`를 공개한다. typed rejection
   channel은 다음 reference-admission Slice를 위해 준비되어 있다. 그전까지
   structured `@`, `$` draft는 실패-폐쇄 상태를 유지한다. 터미널을 소유한
   thread는 provider I/O를 기다리지 않는다.
5. [`AgentRuntime`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/runtime/mod.rs)은
   command 검증, backend 수락, semantic commit, Journal publication 순서를
   보장한다. `StartTurn`과 `SteerTurn`은 correlation이 있는 submission 경계로만
   들어오며 일반 command 경계는 `SubmissionId` 없는 두 command를 거절한다.
   worker가 소유한 durable writer는 text update를 크기가 제한된
   immutable segment로 바꾸고, commit된 record를 공개하기 전에 semantic
   commit을 동기식으로 append한다. 권위 있는 backend snapshot은 이미
   durable한 segment를 수정하지 않고 새 message revision을 시작한다. 아직 segment를
   내보내지 않은 연속 replacement는 같은 unpublished revision을 공유하고, 빈 최종
   replacement는 zero-byte terminal seal로 표현한다.
   provider 관찰 결과도 semantic engine을 통해 변환한 뒤 변경 알림을
   공개한다. 거절된
   command와 잘못된 backend event는 commit된 의미로 기록하지 않지만,
   실패를 닫으며 만들어진 terminal event는 기록한다.
   `AgentSession::transcript_reader`는 같은 Journal에서 크기가 제한된 읽기
   전용 suffix 복사본을 제공하며 내부 lock이나 저장 구조는 노출하지 않는다.
   capacity나 storage 실패가 나면 semantic 결과를 volatile live suffix로
   공개하고 `JournalDurability::Gap`을 유지한다. storage가 다시 write를 받고 열린
   모든 message에 실제 terminal seal이 생기면 같은 writer가 complete snapshot
   하나를 공개한 뒤 incremental commit으로 돌아간다. 빈 message도 zero-byte
   terminal seal을 받고, `ActivityStarted` 뒤 첫 text segment 전에 crash가 나면
   recovery가 interrupted zero-byte seal을 만든다. segment가 없는 empty replacement는
   시간이나 ordering 경계에서 `MessageReset`으로 저장하고, 종료 시에는 zero-byte
   terminal seal로 표현한다. adapter가 semantic `ModelWork`로 승인한 관찰 가능한 plan이나
   reasoning summary도 같은 segment와 seal 경로를 쓴다. yo가 받지 않은 숨겨진 reasoning과
   승인하지 않은 backend-specific Request Audit payload는 이 semantic 경로 밖에 남는다. 공유 observation stream은 각 typed
   durability 전환을 영향을 받는 semantic record보다 먼저 정렬하므로 coalesced worker
   wake-up도 Gap-to-Durable 전환을 지우지 못한다. 같은 level-triggered readiness가
   주기적인 agent poll을 기다리지 않고 terminal owner를 깨운다. CLI adapter는 이 순서를 정확한 cutoff
   종류와 함께 TUI 상태에 전달한다. Chat·status 행·banner 중 어떤 방식으로 표현할지는
   별도 product 계약으로 남긴다. 저장된 Session 검사는 아래의 별도 read-only
   경로를 따른다. 실행 가능한 continuation은 frontend history Projection에서
   상태를 만들지 않고, 아래의 별도 검증된 recovery 경로를 사용한다.
6. [`runner` source scheduling과 redraw](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/unix.rs)는
   회전 cursor로 준비된 terminal·agent·workspace·skill observation 중 한 건씩
   선택한다. 선택한 observation이 TUI 상태를 갱신하며, process termination은
   이 순회 밖의 strict-priority 경로로 남는다. runner는 완성된 `Surface`를 조합해
   활성 presenter로 보낸다. `runner/view.rs`는
   같은 record stream에서 Chat, Transcript, Request를 선택한다. Chat의
   사용자 입력은 `StartTurn` 또는 `SteerTurn` command가 이 순서에 나타난
   뒤에만 표시된다. terminal `EventStream` readiness와 agent·workspace·skill
   producer readiness가 owner thread를 깨운다. 각 live-source trait가 이 계약을
   필수로 요구하며 주기적인 관찰 fallback은 없다. Unix 종료 handler는 durable signal
   bit를 공개하고 nonblocking async-signal-safe write만 수행한다. 일반 notifier thread가
   이 byte를 같은 frontend wake로 바꾼 뒤 host가 정리하고 선택한 원래 signal을 재생한다.
   상태 변경은 event마다 동기적으로
   그리지 않고 frame을 요청한다. `FrameScheduler`는 첫 frame과 resize frame을 즉시
   공개하고, 일반 요청은 `TuiSession` 제한에 맞춰 합친다. 기본은 120fps이고 host가
   `FrameRateLimit::Fps60`을 선택하면 60fps다. readiness나 예약된 frame·motion·활성
   backpressure 마감이 없으면 owner는 무기한 잠들 수 있다. 10ms backpressure 재시도는
   작업이 실제로 보존된 동안에만 deadline으로 남는다. `@`나 `$` discovery를 dispatch하는 editor mutation은 provider
   결과보다 먼저 frame을 요청하고, 이전 usable panel은 pending snapshot gate 뒤에
   계속 보인다. elapsed로 선택한 Rich Braille 또는 ASCII 작업 marker frame을 고정된
   최대 폭 영역에 그리거나 고정 문구 activity sheen을 실제로 그린 Chat
   frame은 보이는 period 중 가장 짧은 값을 반환하고, runner는 터미널 세대 epoch의
   다음 경계를 예약해 event redraw와 합친다. 숨김·좁음·낮음·idle·reduced-motion·zero-size
   indicator는 timer를 활성화하지 않는다. 한 grapheme activity status도 pulse할 수
   있으므로 계속 animated indicator다.

   Inline Chat frame 준비는 두 부분의 transaction이다. `runner/state.rs`는
   완료된 unpublished item의 최대 연속 prefix를 persistent 출력으로
   선택하고, 나머지 transcript suffix·prompt·chrome·overlay만 자연 높이의
   live `Surface`로 조합한다. `terminal/mode/inline`은 persistent 행과 live
   update를 공유 ANSI encoder 이전에 보존되는 typed `TerminalOp` group으로
   compile한 뒤 direct unbuffered Unix transport로 출력한다. effect ledger는
   관찰한 terminal geometry, cursor 범위, addressable prefix, 확정 scroll, anchor가
   정확하지 않은 possible-scroll 상태를 구분한다. 정확한 downstream 진행률이
   complete operation 경계에서 한 번의 clear-and-restart 또는 suffix-resume
   recovery를 허용하며, partial operation·possible scroll·두 번째 실패는 fatal이다.
   복구된 correction은 bounded `TuiSession` 환경 증거로 보존한다. write와 flush가
   모두 완료된 뒤에만 publication cursor가 전진한다. 이어 presenter가 대기 중인
   resize 알림을 drain하고 terminal size를 새로 읽는다. size나 geometry epoch가
   stale이면 persistent acknowledgement는 유지하되 준비한 live geometry는 버리고
   suffix만 즉시 다시 준비한다. tail에서 떨어진 Chat viewport, Transcript,
   Request는 publication을 멈추고, Fullscreen은 cursor를 사용하지 않는다.

승인된 순서, 중단 gesture, 정직한 status 데이터, 반응형 맞춤 정책은
[정적 입력 chrome 계약](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.chrome.input-stack.md)이
소유한다. 이 runtime에서 `shell::chrome`은 활성 상태와 `TuiSessionInfo`로
typed 행을 계산하고 폭에 맞춘다. `shell::chrome::help`는 label을 개행하거나
자르는 대신 우선순위가 낮은 action 전체를 제거한다. 공용
`input::key_notation` formatter는 설정된 semantic binding에서 `Esc`, `^C`,
`^D`, `S-Enter` 같은 terminal 관례 표기를 만들지만, action이 현재 사용
가능한지는 결정하지 않는다. `shell`은 그 영역을 prompt 주변에 조합하고,
`input::control`은 아주 작은 frame이 시각 안내를 표시하지 못해도 mapping된
interrupt intent를 dispatch한다.
80ms marker frame 순서, 최대 폭 marker 영역, 연속 2초 shimmer, 설정 가능한 120/60fps runner frame 경계는
[activity motion profile](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.activity-motion-profile.md)과
[activity motion scheduling](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.activity-motion-scheduling.md)
계약이 소유한다.

change lane은 command나 event 내용을 싣지 않으며 용량은 하나다. 따라서
여러 commit이 읽지 않은 알림 하나로 합쳐져도 이력은 사라지지 않는다.
구체적인 local reader가 Journal sequence를 따라 당시 확인한 head까지
계속 읽기 때문이다. backend가 최종 실패해도 adapter는 Journal에 이미
확정된 실패 record를 먼저 모두 공개한 뒤 연결 오류를 보고한다.

Codex와 Grok wire JSON 및 provider identifier는 각 backend adapter 밖으로 나오지 않는다.
터미널 input event와 rendering type은 `yo-tui` 밖으로 나오지 않는다.
그 사이를 지나는 command와 event type은 `yo-core`가 소유한다.

## 저장된 Session 검사

저장 history는 live startup 경로에 들어가지 않는다.

```text
yo session [--all] [--details]
  ↓ 기존 Host identity와 repository를 만들지 않고 읽기
LocalSessionReader::discover
  ↓ 검증된 tail summary
workspace로 거른 metadata table을 stdout에 출력

yo session SESSION_ID [--view chat|transcript|request]
  [--limit N] [--content none|preview|full] [--ascii]
  ↓ writer lease 없는 한 시점의 physical snapshot
yo-core read_stored_session
  ↓ envelope 검증 + Journal recovery
StoredSessionHistory
  ↓ Chat, bounded Transcript 또는 Session 전체 Request correlation Projection
  ↓ 정확한 Journal 경계, reader가 없으면 Request Audit을 명시적으로 unavailable 처리
yo-tui archived Projection
plain stdout

yo usage SESSION_ID [--ascii]
  ↓ 같은 읽기 전용 snapshot과 검증된 복구 경로
StoredSessionHistory
  ↓ typed 완료 usage 영수증과 coverage-aware aggregate
yo-tui archived Usage Projection
plain stdout
```

`yo-cli/src/command.rs`는 command 문법, `session.rs`는 선택과 table/output routing,
`config.rs`는 날짜 형식 설정, `storage.rs::open_default_reader`는 writer startup과
분리된 읽기 전용 조합을 소유한다.
Request에는 anchor selector가 없다. 인접 request를 추측하지 않고 durable correlation과
availability record 전체를 Journal 시간순으로 출력한다. 이 Projection은 backend payload나
physical repository envelope를 출력하지 않는다.

`--limit`과 `--content`는 Transcript 전용 selector이며 Chat·Request·Session 목록에서는
command parser가 거부한다. 양수 limit은 최신 semantic record를 고른 뒤 원래 Journal 번호를
보존한 채 시간순으로 출력한다. selector를 생략하면 기존의 complete Transcript를 그대로
유지한다. `content=none`은 payload type과 원래 UTF-8 byte 길이만 남기고, `preview`는 extended
grapheme cluster를 자르지 않으면서 최대 256 UTF-8 byte를 남기며, `full`은 전체 payload를
유지한다. 별도 Usage 명령은 typed Session usage Projection을 재사용하므로 CLI에서 영수증을
다시 해석하지 않고 backend나 live TUI view도 시작하지 않는다. `--ascii`는 출력 glyph
profile만 바꾼다.

stdout이 terminal이면 `session.rs`가 관찰한 폭, Session 전용 열 우선순위와
continuation hint를 범용 `yo-tui::plain` renderer에
전달한다. 먼저 PATH와 DETAIL, 다음으로 continuation/version, 시작 시각, workspace를
접는다. 짧은 label/value pair는 주 행 아래를 왼쪽부터 채우고, 다음 pair 전체가
들어가지 않을 때만 다음 줄로 옮긴다. PATH와 DETAIL은 진행 중인 flow를 끝내고
독립된 한 줄을 사용한다. label/value pair가 전체 폭에 들어가면 같은 줄에 두고,
부족할 때만 개행하는 label block으로 바꾼다. 너무 긴 flow pair도 같은 block 형태로 승격한다.
접힌 상세가 있는 record 사이는 빈 줄 하나로 구분한다. 고정된
identity/status/updated 시각도 들어가지 않으면 공유 table header를 없애고 모든
필드를 label이 있는 세로 card로 바꾼다. 접힌 값은 terminal grapheme cell 경계에서 개행하며
잘라내지 않는다. 하나의 atomic grapheme이 terminal 전체 폭보다 넓으면 쪼개거나
버리지 않고 명시적으로 실패한다. terminal의 heading은 왼쪽 끝에서 굵게 표시하고
값만 두 cell 들여쓴다. stdout이 terminal이 아니면 파이프와 파일 결과가 terminal
폭에 따라 달라지지 않도록 ANSI style이 없는 한 줄 표를 유지한다.

선택 설정 파일은 읽기만 하고 만들지 않는다. Linux는
`${XDG_CONFIG_HOME:-$HOME/.config}/yo/config.yaml`, macOS는
`$HOME/Library/Application Support/yo/config.yaml`을 사용하며, `YO_CONFIG`로
명시적인 경로를 고를 수 있다. 현재 pre-version schema는 다음과 같다.

```yaml
session:
  list:
    date_format: "%Y-%m-%d %H:%M %:z"
tui:
  max_fps: 120
```

`config.yaml`은 일반 Session·TUI 설정만 소유한다. 최상위 `model` field는 알 수 없는
field다. 모델 정의, catalog seed, startup preference의 durable owner는
`connections.yaml` 하나다.

`yo connect --from /absolute/definition.yaml`은 임시 grouped definition 하나를 읽는다.
정확한 `--from -`은 같은 shape를 표준 입력에서 읽는다. 문서는 Provider·Account 하나,
endpoint 하나, 필수 base-profile mapping 하나, 모델 1~4,096개를 선언한다. 이 mapping은
일부 field만 갖거나 비어 있어도 된다. 모델은 닫힌 profile field를 전체 단위로 교체할 수
있고 생략한 field는 base 값을 상속한다. 구조화 mapping은 재귀 merge하지 않으며, 이렇게
해석한 각 model profile은 완전해야 한다.

```yaml
provider: example
provider_display_name: Example
account: team
account_display_name: Team
base_url: https://api.example.test/v1
profile:
  api_dialect: openai-responses
  tokenizer_profile: utf8-bytes/v1
  input_token_limit: 1000000
  max_output_tokens: 65536
  reasoning_parameters:
    effort: medium
  optional_request_parameters: {}
  tool_capability_policy: local-tools/v1
models:
  - model: model-a
    model_display_name: Model A
  - model: model-b
    profile:
      api_dialect: openai-chat-completions
      max_output_tokens: 8192
```

Import는 해당 Provider·Account group 전체를 원자적으로 교체하며 빠진 기존 모델을
제거한다. 임의 default를 선택하지 않는다. 기존 preference가 교체에서 제거된 모델을
가리킬 때만 함께 clear한다. Preview·확인·credential capture는 group 전체에 각각 한 번만
수행한다. Preview는 account metadata 전체와 정확한 catalog 또는 discovery seed를
비교하고 추가·변경·제거되는 모델을 명시하며, 변경되거나 제거되는 complete binding을
사용하는 저장 Session은 그 정확한 binding이 복원될 때까지 재개되지 않을 수 있음을
경고한다. Non-interactive 형식은 absolute `PATH`를 쓰는 `--from PATH` 또는 정확한
`--from -` 중 하나와 absolute `--credential-file PATH`, `--yes`를 요구하며 어느
YAML에도 secret을 넣지 않는다.

Release 시점에 알려진 QwenCloud 또는 Kimi catalog는 `base_url`, `profile`, `models`
대신 `catalog`를 사용한다.

```yaml
provider: qwencloud
provider_display_name: QwenCloud
account: team
account_display_name: Coding Team
catalog: qwencloud-coding-plan-intl/v1
```

닫힌 QwenCloud ID는 `qwencloud-coding-plan-cn/v1`,
`qwencloud-coding-plan-intl/v1`, `qwencloud-token-plan-team-intl/v1`이다. Kimi는
`kimi-platform-ai/v1`, `kimi-code-membership/v1`을 받는다. 저장 seed는 connect
candidate를 만들지만 startup에서 route할 binding을 직접 만들지 않는다. Candidate를
선택하면 필요한 explicit private-replay 동의를 포함한 complete profile을 저장한다.
Catalog identity나 ModelId만으로 그 동의를 만들 수 없다.

OpenRouter discovery는 explicit shape에서 `models`만 생략하며 이를 허용하는 유일한
Provider다. 저장 seed가 bounded authenticated picker에 endpoint와 base profile을 제공한다.

날짜 문법은 strftime과 호환되고 UPDATED와 STARTED 모두 보는 머신의 local
timezone으로 표시한다. `tui.max_fps`는 숫자 `60` 또는 `120`만 받으며 live startup에서
한 번 읽어 보존되는 TUI 세대에 적용한다. 실행 중 reload는 지원하지 않는다. Whole-field
YAML null, 알 수 없거나 중복된 field, 중복 ModelId, 불완전한 profile, relative `--from`
경로는 credential capture나 mutation 전에 실패한다. `{}`는 구조화 field를 빈 mapping으로
교체하고 그 아래의 null은 구조화 값으로 유지한다. Plain YAML 1.1
`y`/`yes`/`true`/`on`과 `n`/`no`/`false`/`off`는 대소문자와 무관한 boolean이고,
`1_000`은 정수 `1000`이다. Quoted 형식은 string으로 남는다. startup과 native resume은
producer가 complete profile을 저장하므로 startup과 native resume은 authored inheritance를
다시 수행하지 않는다. 설정 파일이 없으면 built-in Session/TUI 설정은 유지하지만 startup
target은 제공하지 않으므로 live startup은 setup 안내를 표시한다. 파일을 읽을 수
없거나 retired field/알 수 없는 field/크기/date
format/frame rate가 잘못되면 조용히 기본값으로 대체하지 않고
명시적으로 실패한다. reader는 no-follow nonblocking descriptor 하나를 열어 regular file인지
확인하고 안정적인 identity와 metadata를 capture하며 1 MiB와 판별용 한 byte까지만 읽으므로
FIFO가 command를 멈추거나 동시에 커지는 파일이 상한을 우회하지 못한다. Preference mutation은
이 파일을 다시 capture하고 public commit 전에 정확한 command-local snapshot이 바뀌지 않았음을
요구한다. 모델 API key는 환경 변수에서 읽지 않는다.
저장한 모델을 선택하면 Yo는 선택된 `config.yaml` 옆의 별도
`credentials.yaml`을 다음 Provider-Account 순서의 현재 pre-version 구조로 읽는다.

```yaml
providers:
  openrouter:
    default:
      api_key: "..."
  qwencloud:
    default:
      api_key: "..."
```

credential 파일은 현재 사용자가 소유한 regular file이어야 하고 group/other 권한 bit가
없어야 한다(보통 mode `0600`). 서로 다른 Provider 아래에는 같은 Account ID를 사용할
수 있으며 선택된 Provider·Account exact pair만 resolve한다. Revision이 없는 current-shape
파일도 snapshot으로 계속 읽을 수 있다. `LocalCredentialRepository`는 private store
lock 아래에서 파일을 다시 읽고 candidate secret을 보관하지 않은 채 정확히 한 pair의 add,
replace 또는 remove를 준비한다. Commit은 add 또는 replace일 때만 candidate를 받고, 관련
없는 pair를 모두 보존하며, 독립적으로 만든 private `crev-...` receipt가 든 완전한 mode
`0600` snapshot을 원자적으로 게시한다. 예정 receipt와 exact pair 상태가 같으면 반복
commit은 idempotent하고, 관찰한 revision이 다르면 conflict다. 마지막 pair를 제거해도
`absent`로 돌아가지 않고 revisioned empty file을 남긴다. 이 write는 core storage
boundary다.

`config.yaml`, `credentials.yaml`, `connections.yaml`, operation journal은 `yo-yaml`을
공유한다. 문서 하나만 허용하고 구조·replay 예산을 유한하게 제한하며, 제한 안의 작은 alias만
허용하고 duplicate key·merge key·unknown alias·cycle·추가 문서를 거절한다. 네 문서에는
top-level format-version field가 없다. 알 수 없는 `version` field나 journal의
`profile_digests` field는 일반 unknown field이며 mutation 전에 typed decoding에서 실패한다.
Yo는 과거 pre-release shape를 별도로 분류하거나 decode·migration·dual write·downgrade하거나
자동 삭제하지 않는다.

Sibling `connection-operation.yaml`은 credential과 public repository를 함께 바꾸는
operation의 secret-free durable intent를 소유한다. 현재 pre-version record는 불투명 operation
ID, 정확한 expected·planned public revision과 크기가
제한된 완전한 prospective public snapshot, add·replace·remove·preserve 중 하나인 credential
receipt, legal phase 하나를 담는다.
`ApiCredential`, candidate identity, verification payload는
받을 수 없다. 파일이 없을 때 capture는 아무것도 만들지 않으며, 첫 intent publication은
exclusive다. 각 phase replacement는 크기가 제한되고 mode `0600`, no-follow,
current-user-owned이며 durable·atomic하고 exact entry를 확인한다.
모든 journal mutation은 같은 repository directory의 mutable
`LocalConnectionOperationGuard`를 요구한다. Guard의 nonblocking file lock은 capture와
publication 사이에서 두 번째 process-equivalent owner를 배제하고, mutable borrow는 guard
하나를 공유하는 동시 mutation call을 막으며, 다른 directory의 guard는 journal byte를 바꾸기
전에 실패한다.

`plan_connection_recovery`는 순수 state-table boundary다. Connect는 commit되지 않은
expected/expected intent를 abandon하고 credential이 exact planned receipt에 도달한 뒤 public
CAS만 재개하며, exact planned public byte에서만 complete한다. Disconnect는
expected/expected를 abandon하고 public snapshot이 exact planned가 된 뒤에만 준비한 remove를
commit하거나, exact credential revision을 mutation 없이 preserve한다. Repository 사실보다
앞선 phase, credential-first disconnect, 다른 public winner, 계약에 없는 모든 상태는 private
credential revision을 노출하지 않는 typed conflict다.
`LocalConnectionOperationRepositories`는 shared operation lock을 얻기 전에 absolute normalized
path, lexical directory 하나, 닫힌 sibling 파일명 세 개만 허용하고 symbolic-link component를
거절한다. 없는 state directory는 사용자 전용 mode로 만든 뒤, session이 lock 획득 전에
directory의 device와 inode를 capture하고 획득 직후 같은 pathname identity인지 검사한다. 이어
각 journal·repository capture와 effect 전에 pathname
component와 directory identity를 다시 검사한다. 따라서 검사하는 틈의 directory replacement나
symbolic-link retarget은 그 다음 mutation 전에 실패한다. 이 fail-closed pathname 재검증은
적대적인 ABA replacement를 막거나 검사와 뒤따르는 filesystem call을 하나로 묶는 원자적
directory-descriptor anchor라고 주장하지 않는다. Session은 journal을 capture한 뒤 state
table이 정한 결정만 실행한다. Commit되지 않은 intent는 abandon하고, 뒤처진 phase는 정확한
다음 repository CAS 앞뒤로 따라잡으며, repository pair가 이미 완료된 경우에는 `complete`까지
전진한 뒤 exact journal을 지운다. Connect recovery는 secret을 재구성하거나 commit하지 않는다.
Disconnect remove는 candidate 없이 commit하고 preserve는 credential mutation boundary를
호출하지 않는다. Repository와 journal 오류는 private credential revision을 투영하지 않고
안전한 operation kind, action, phase만 유지한다. External connect는 이제 준비와 commit에 같은
held session을 사용한다. External disconnect도 선택한 저장 target 하나에서 같은
Provider·Account credential action을 묶고 어떤 credential 제거보다 public 제거를 먼저 commit한다.

`yo connect qwencloud:Account`는 그 Account의 `connections.yaml` 저장 QwenCloud catalog seed를
해석하고 credential을 읽기 전에 같은 controlling-TTY picker를 연다. Release 시점에 알려진
row는 모두 보이며 Yo가 지원하지 않는 row는 사유와 함께 disabled 상태가 된다. 취소하거나
disabled row를 고르면 credential을 읽지 않고 intent나 repository mutation도 만들지 않는다.
정확한 `yo connect qwencloud:Account:Model`은 picker를 건너뛰며, 닫힌 catalog 밖의 Model은
`yo connect --from`으로 저장 definition을 교체하라는 안내와 함께 실패한다. 원격 model-list 요청은 없다.
선택 가능한 row 하나를 고른 뒤에는 구조적 binding admission, preview, credential capture,
journal, commit 경로가 그대로 권위 경계다. 등록 자체는 모델 요청을 보내지 않으며 해당
account가 선택한 row를 사용할 수 있다고 주장하지 않는다.

`yo connect kimi:Account`는 candidate key 하나를 읽고 저장 Kimi 제품 seed의 endpoint에서
bounded 인증 `GET models` snapshot 하나를 가져와 normalize한 typed row를 같은 picker로
넘긴다. 첫 valid exact ModelId가 이기며 4,096개보다 많은 행은 snapshot 전체를
거부한다. Platform은 검토된 K3, K2.7 Code, K2.7 Code Highspeed, K2.6 envelope만
허용한다. Code Membership은 정확한 `k3`, `k3-256k`, `kimi-for-coding`,
`kimi-for-coding-highspeed` envelope를 허용하며 `k3-256k`를 recommended로 표시한다.
Cross-product와 future row는 숨기지 않고 안정적인 disabled 이유와 함께 표시한다. 각
행은 remote context와 reasoning 근거가 그 제품의 검토된 envelope 안에 있을 때만
선택할 수 있다. K3/K2.7 저장 binding을 게시하기 전에 compact preview는 bounded
Kimi assistant state를 현재 사용자 로컬 Session record에 암호화하지 않고 보관한다고
알린다.

Secret-free connection preparation은 닫힌 Kimi catalog/profile compatibility
검사를 유지해 잘못된 cross-product나 limit 행을 credential 또는 public-state 변경 전에
거절한다. 이 검사는 Kimi wire 값을 만들지 않으며 Connector의 독립적인 client 생성 전
검증을 대신하지 않는다.

그 뒤 flat `yo-connector-kimi` crate가 선택된 complete binding의 exact request, stream,
provider-private assistant codec, visible projection, encoded-size 문법을 소유한다.
Platform은 기존 request shape를 유지한다. Code K3는 허용된
reasoning effort와 preserved-thinking `keep: all`을 보내고 Code K2.7은 forced
preserved thinking을 보낸다. 두 Code 계열은 opaque `prompt_cache_key` 하나도 보낸다.
Backend는 Session identity에서 hint를 한 번 만들고 Provider 분기 없이 일반 요청과 재개
요청에서 재사용하며, 직렬화 여부는 Connector만 결정한다. Hint는 redacted되고
binding identity, replay evidence, log, diagnostic, Transcript, trace가 되지 않는다.
성공한 K3/K2.7 round는 Kimi payload 안에 완전한 reasoning, content, tool-call message를
담은 bounded opaque provider-private envelope 하나를 낸다. Core는 그 payload를 해석하지
않는다. 이 항목은 frontend와
Request-trace projection에서 숨기고 visible assistant/function projection 옆에 atomically
저장하며, completed neutral projection, exact private replay profile schema, binding epoch가
일치한 뒤에만 허용한다. Physical Journal member 순서와 profile string은 바뀌지 않는다.
Managed loop는 완료된 private-profile assistant-and-calls group마다 envelope 하나를 정확히
요구하고 recovery도 Continuation Anchor를 재구성하기 전에 같은 순서를 검증한다. 다음 Kimi
request는 그 visible group을 private assistant message 하나로 정확히 한 번 교체한다.
Semantic-only binding은 private item을 저장하거나 replay할 수 없고 incomplete 또는 failed
round는 private Continuation Anchor를 만들지 않는다.

`yo connect openrouter:Account`는 정확한 저장 seed에 normalized endpoint와 complete base
profile이 있을 때만 대화형 discovery target이다. Recovery와 snapshot capture 뒤 Yo는 no-echo
candidate key 하나를 읽고 endpoint prefix에 `/models/user`를 더한 주소로 인증 `GET`을 보낸다.
요청은 same-origin redirect 수와 connect, attempt별 response-header, body inactivity, absolute
deadline이 제한되고 성공 응답은 bounded JSON이어야 한다. Core normalization은 capability나
profile 때문에 unavailable인 row도 exact Model ID가 valid하면 첫 row를 유지한다. Capability 배열은
중복이 없고 순서와 무관한 set으로 다루며 text input과 output을 요구한다. 설정한 local-tools
policy는 remote `tools`나 `tool_choice` capability 중 하나라도 없으면 no-tools로 좁힌다. Authored
model override가 없는 field에만 valid remote context limit을 적용한다. 정확한 우선순위로 typed
disabled 이유 하나를 고르고 enabled row만 선택 가능한 complete binding을 가진다.
Controlling-TTY picker는 name과 ID를 검색하고 한 번에 최대 여덟 result row를 보여 주면서 disabled
row와 그 이유를 포함한 모든 match에 scroll로 도달하게 하며 disabled 선택은 막는다. 선택, 취소,
input·render 실패, unwind에서 terminal mode, cursor,
dynamic panel을 한 cleanup owner가 복원한다. Remote string은 terminal 출력 전에 printable하고
되돌릴 수 있는 byte escape 경계를 지난다. 검색 편집은 backspace 한 번에 extended grapheme
cluster 하나를 지우고, 줄바꿈과 자르기는 byte나 scalar 개수가 아니라 terminal cell 폭을 쓴다.
Bounded raw-key decoder는 완전한 CSI 또는 SS3 sequence를 소비해 plain·modified Up/Down을
해석하며, 지원하지 않거나 malformed·overlong인 sequence tail을 검색 text로 남기지 않는다.
부분 escape와 UTF-8 scalar는 유한한 read deadline을 가지며, 잘못된 UTF-8 continuation이 그
자체로 독립적인 key라면 그 byte를 다음 decode에 보존한다. 선택 뒤에는 기존 concise connection
preview로 들어가고
`--verbose`는 그 preview만 확장한다. 취소는 새 intent나 repository mutation을 만들지 않으며,
discovery에 쓴 같은 in-memory key는 마지막 구조적 admission 뒤 게시할 때까지 유지된다. 두 부분
discovery는 `--credential-file`과 `--yes`를 거절한다.

`yo connect Provider:Account:Model`은 capture한 저장 definition이나 검토된 저장 catalog
seed의 exact reference 하나를 받는다. Prospective 집합은 해당 Provider·Account의 저장 sibling과
선택한 complete binding으로 만든다. 선택한 coordinate의 이전 binding은 등록 개수에서 제외하고
verbose preview에서 이전 profile과 새 profile을 비교할 때만 남길 수 있다. Prospective 저장
upsert는 secret을 읽기 전에 startup-policy admission을 통과해야 한다. Yo는 확인을 받은 뒤 controlling TTY에서만
크기가 제한된 API key 하나를 읽는다. Credential capture는 `ISIG`를 유지하고 `ECHO`와
`ICANON`을 끄며 `VMIN=1`, `VTIME=0`으로 설정한 뒤 정확한 원래 terminal 설정을 복구한다.
명시적 복구가 오류를 반환하면 보존된 guard가 unwind 중 복구를 재시도한다.
줄 단위 controlling-TTY prompt에는 모두 16,384-byte 입력 한계가 있다. 초과하면 Yo는 한계
오류를 반환하기 전에 아직 읽지 않은 terminal input queue를 비우고, queue flush 실패는 별도
오류로 보고하며, credential prompt에서는 어느 오류든 반환하기 전에 echo를 복구한다.
External exact target은 대신 `--credential-file PATH --yes`를 사용할 수 있다. 두 option은
반드시 함께 있어야 하고 `--yes`는 interactive `--verbose` view와 충돌하며 Local Codex는 파일을
열기 전에 이 조합을 거절한다. Recovery와 exact plan 준비 뒤 이 경로는 확인을 생략하고 final
credential path를 no-follow로 한 번만 연다. 현재 사용자 소유 regular file이면서 mode가 정확히
`0400` 또는 `0600`인 경우만 받고, 16,386-byte 안정 metadata 경계 안에서 EOF까지 읽은 뒤 마지막
LF 또는 CRLF 하나만 제거하고 16,384-byte UTF-8 `ApiCredential` 규칙을 적용한다. Capture 실패는
새 intent나 repository mutation을 만들지 않고 TTY로 fallback하지 않으며 source
file을 바꾸거나 노출하지 않는다. 새 plan 전에 recovery가 이전 operation을 이미 완료했을 수는
있다. 환경 변수, secret argument 값, standard input, child process, config file은 credential
channel이 아니다.
확인은 먼저 complete preview를 memory에 만든다. 그 preview와 prompt를 게시하기 직전에
controlling-TTY의 대기 입력을 비워 그 뒤 새로 들어온 line만 plan을 승인할 수 있게 한다.
Flush 실패는 prompt 게시나 repository mutation 전의 별도 치명적 input-boundary 오류다.
Noninteractive `--yes`는 별도의 captured-plan 승인 경로로 유지된다. 확인 화면은 선택 target을
먼저 보여주고, 안정적인 의미 plan marker(`+`, `~`, `−`, `=`)로
생성, 변경, 제거, 유지 효과를 구분한다. 기본 화면은 판단에 필요한 이 변경 집합을 유지하고,
credential action에 Provider와 Account를 한 번만 표시한 뒤 그 account에 등록할 각각의 정확한
Model ID를 한 번씩 나열하며, 간결한 plan 개수로 끝난다. `-v` 또는 `--verbose`는 model을
제외한 connection field와 resolved profile field가 정확히 같은 model을 한 그룹으로 묶고,
공유하는 secret-free endpoint, dialect, profile field를 한 번만 표시한다. 어느 field든 다르면
별도 profile group을 만들므로 압축 때문에 서로 다른 binding 동작이 가려지지 않는다. 일반적인
Model ID는 그대로 표시하고, 목록 구분자나 모호한 공백·따옴표 문자가 든 ID는 되돌릴 수 있는
JSON string 따옴표로 표시한다. 항목과 구분자가 inline 목록 폭에 함께 맞지 않으면 맞는 ID를
자르거나 구분자만 다음 줄에 두지 않고 별도 bullet 행으로 전환한다. Credential 행은
repository가 준비한 action에서 파생하므로 새 key 추가와 기존 key 교체가 오해를 부르는 같은
문구를 쓰지 않는다. Check 표시가 있는 성공 요약으로 command를 끝낸다. Preview는 controlling-TTY
폭을 쓴다. Success rendering은 standard output을 한 번 snapshot하여 terminal이면 0이 아닌
열 수를 쓰고, 폭을 읽을 수 없거나 0이면 80열로 fallback한다. Redirect된 output은 결정적인
평문이며 `NO_COLOR`도 terminal output을 평문으로 유지한다. 두 경로 모두 terminal-safe한 폭
0이 아닌 grapheme을 자르거나 분리하지 않고 직접 줄바꿈하여, shell의 우연한 줄바꿈에 의존하지
않으면서 secret이 아닌 값의 exact bytes를 보존한다. 두 셀 atomic grapheme을 1열에 표시하려면
typed width 오류로 실패한다. Complete success output은 첫 operation commit 전에 준비하므로
표시 실패가 이미 commit된 상태를 command 실패처럼 보이게 만들지 않는다.

Command는 candidate key로 모델 요청을 보내지 않는다. 확인 뒤 capture한 config를 다시
검사하고 secret-free intent를 게시한 뒤 exact add 또는
replace credential을 commit한다. 이어 journal을 전진시키고 exact 저장 public snapshot을
게시하며 complete까지 전진한 뒤 journal을 지운다. Authentication, entitlement, request
acceptance는 일반 모델 사용에서만 확인한다. Credential commit 뒤 crash가 나면 저장된 public
byte만 재개하고 secret을 재구성하거나 사용하지 않는다.

`yo disconnect`는 대화형으로 유일한 저장 target을 추론하거나 capture한 정확한
`Provider:Account:Model` reference 하나를 입력받는다. 자동 실행은
`yo disconnect PROVIDER --account ACCOUNT --yes`를 요구하며 해당 pair에 저장 model이
정확히 하나일 때만 진행한다. `--yes`는 여러 model 중 하나를 추측하지 않는다. 확인 전에
Yo는 같은 capture snapshot에서 prospective 저장 removal을 만든다. 간결한 기본 preview는
같은 의미 plan marker로 저장 removal, default와 API-key 변경, 새
Session과 저장된 Session에 미치는 영향을 보여 주며, API-key 행은 이미 표시한 Provider·Account
문맥에서 그 key를 계속 사용하는 모든 정확한 Model ID를 표시하고 모호한 ID에는 같은 방식의
되돌릴 수 있는 따옴표를 쓴다. `-v` 또는 `--verbose`는 정확히 제거할 complete binding,
source, 같은 pair에 남는 binding을 추가로 보여 준다. Prospective startup layer를 실제로
해석해 새 Session이 사용할 정확한 낮은 우선순위 target을 이름으로 보여주거나 남는 target이
없다고 알리며, preference 제거만 보고 동작을 추측하지 않는다. 남은 account model은 명시된
account 문맥 안의 정확한 Model ID로 표시해 제거 profile 전체를 반복하지 않는다. 같은
controlling-TTY 폭 경계가 모든 preview row를 관찰한 폭 안에 둔다. Check 표시 success target과 verbose의
remaining-model bullet은 preview와 같은 reversible remote-text 및 ambiguous-item 표시 경계를
지난다. 같은 pair의 model이나
catalog seed가 하나라도 남으면 credential을 보존한다. 제거 뒤 dependent set이 비었을 때만
credential 제거를 준비하며, credential이 이미 없으면 상태를 꾸며내지 않고 intent 전에
실패한다. 확인과 마지막 config guard 뒤에는 secret-free intent를 게시하고 public 제거를
commit하며 `public_committed`로 전진한다. 필요한 경우에만 credential을 제거하고
`complete`까지 전진한 뒤 journal을 지운다. 기존 Session history는 삭제하지 않지만, exact
binding을 복원하지 않으면 제거한 complete binding에
귀속된 Session이 native resume되지 않을 수 있다. Preview는 이 continuation 결과를 저장
history 보존과 구분해 보여 준다.

endpoint, model, API dialect, 파생된 connector identity, resolved profile과 표시 이름은
secret-file content가 아닌 binding data로 둔다. 위 catalog의 limit과
Model ID는 운영자가 관리하는 예시이며 현재 Provider의 정확한 제공 목록과 대조해야 한다.
`utf8-bytes/v1`은 직렬화한 전체 request의 UTF-8 byte마다 token 하나를 세는 보수적인
profile이다. `o200k_base/v1`은 실제 tokenizer가 o200k와 호환되는 binding에만 쓸 수
있고 모르는 profile은 startup에서 실패한다. `max_output_tokens`는 optional known profile hard
maximum이다. unknown이면 producer가 생략하고 whole-field `null`은 잘못된 값이며, absence는
base/model resolution과 durable complete-binding identity에 그대로 남는다. known-cap round는
hard maximum 이하의 양수 request-local 값을 선택하고, 정확히 다시 계산한 connector payload의
input과 cap 합이 input limit에 맞을 때만 admission한다. unknown-cap round는 dialect output field를
생략하고 exact input count가 input limit보다 작아야 하며, 닫힌 Kimi profile처럼 wire 계약이
known cap을 요구하는 connector는 unknown을 거절한다. 첫 explicit runtime은
빈 reasoning mapping 또는 `none`, `minimal`, `medium`, `high` 중 하나인 `effort`를
지원하며, 빈 `optional_request_parameters`와 `local-tools/v1`을 요구한다. 다른 검증된 profile identifier는 설정으로 읽을 수
있지만 그 runtime 동작이 구현될 때까지 startup에서 실패한다.

공개 sibling `connections.yaml`은 일반 `config.yaml`, secret인 `credentials.yaml`과
분리된다. 저장 account, complete model profile, catalog·discovery seed, selection이 소유하는
preference의 유일한 owner다. 다음은 대표 snapshot이다(불투명 revision
값은 예시다).

```yaml
revision: rev-0123456789abcdef0123456789abcdef
preference:
  kind: model
  provider: qwencloud
  account: default
  model: qwen3.8-max
bindings:
  - provider: qwencloud
    account: default
    model: qwen3.8-max
    model_display_name: Qwen 3.8 Max
    connector: openai-responses
    base_url: https://example.test/v1
    profile:
      api_dialect: openai-responses
      tokenizer_profile: utf8-bytes/v1
      input_token_limit: 262144
      max_output_tokens: 8192
      reasoning_parameters: { effort: medium }
      optional_request_parameters: {}
      tool_capability_policy: local-tools/v1
    last_failure:
      kind: rate_limited
      observed_at: 2026-08-17T09:10:11Z
accounts:
  - provider: qwencloud
    provider_display_name: QwenCloud
    account: default
    account_display_name: Default
catalogs:
  - kind: built_in
    provider: qwencloud
    account: default
    catalog: qwencloud-token-plan-team-intl/v1
```

`last_failure`는 complete binding identity에 포함되지 않고 routing을 금지하지 않는 optional
warning-only observation state다. 실제 native model 사용은 secret, request body, response body,
Provider 원문 오류를 보존하지 않고 닫힌 typed outcome 하나를 보고한다. 저장 failure에는
`kind`와 UTC whole-second canonical `observed_at`만 있으며 다음 성공한 model request가 이를
제거한다. 인증, 권한, exact-model availability, rate limit, 그 밖의 request 거부, Provider
availability, transport, timeout, protocol, 설정한 response limit, local binding·credential
전제조건 failure를 서로 다른 kind로 둔다. 사용자 취소, local-tool failure, cleanup failure는
observation을 만들지 않는다.

Request는 자신이 사용한 exact complete binding과 private credential revision을 유지한다.
Request가 끝나면 connection owner가 같은 operation lane에 잠깐 들어가 pending connection
operation을 복구하고 두 repository를 다시 읽는다. 그 binding과 credential revision이 여전히
현재 값일 때만 `connections.yaml` CAS 하나를 게시한다. 따라서 binding 제거·교체나 key
rotation 뒤의 오래된 outcome은 버린다. Observation 저장 failure는 별도로 보고하며 원래
request outcome을 바꾸지 않는다. Capture한 failure는 이후 model-picker snapshot에서
warning으로 표시하지만 해당 행을 disable·숨김·후순위 처리하지 않는다.

Activation은 저장 binding마다 compatible optional durable field 하나를 사용한다. Field가
없으면 enabled이며 이것만 enabled encoding이다. 정확한 `enabled: false`만 disabled를
뜻하고 present true, null, non-boolean 값은 invalid다. 따라서 이전 binary는 all-enabled
snapshot을 읽고 disabled binding이 있는 snapshot은 unknown field로 거절하며, enable이
field를 제거한 뒤에는 다시 읽을 수 있다.

파일이 없으면 canonical unset snapshot이며 디렉터리를 만들지 않고 읽는다. Capture는 모르는
field, 중복 account나 binding coordinate, 대응 account가 없는 binding, 모순된 Provider 표시
metadata, 올바르지 않은 complete binding, 범위를 벗어난 quote 없는 structured-profile 숫자를
거절한다. Optional field도 whole-field null을 거절하며 nested null과 quote한 숫자 모양 string은
정확한 구조화 variant로 유지한다.

Exact-target connect는 complete model 하나를 추가하거나 교체하고 저장 sibling을 보존한다.
Grouped import는 catalog seed를 포함한 Provider·Account definition 전체를 revision 하나로
교체한다. 저장 removal은 exact model 하나를 지우고 같은 pair의 sibling이나 seed가 남아 있으면
account와 credential을 유지하며 exact matching ModelTarget preference만 clear한다.
Preference-only 준비는 모든 저장 definition을 보존한다. 모든 mutation은 새 불투명 revision
하나를 예약하고 기존 old-or-exact-new CAS를 사용한다. 파일이 없을 때 첫 write는 같은 디렉터리
exclusive publication, 이후 write는 durable atomic replacement를 사용한다. 계획한 revision과
byte가 정확히 같으면 idempotent success이고 다른 revision은 conflict다. Credential을 바꾸는
connect나 import는 표시되는 definition이 같아도 복구가 구별할 새 public revision을 예약한다.

모든 live startup은 `config.yaml`과 `connections.yaml` snapshot 하나를 capture한다. Snapshot이
model catalog와 preference를 직접 제공하며 manual/stored composition이나 provenance conflict
경로는 없다. 초기 선택, resume matching, live model picker는 같은 complete 저장 profile을 쓴다.

`yo default TARGET`, `yo default --unset`, `yo model enable TARGET`, `yo model disable TARGET`,
명시적 `yo connect host:codex` 또는 `host:grok`, external model connect, external model disconnect는
nonblocking process operation lock 하나를 사용하고 새
command configuration을 읽기 전에 pending multi-repository work를 해결한다. Preference-only command는 target admission 또는
local delegated-host 검증과 마지막 configuration guard 뒤 public CAS 하나를 게시하고, 새 operation
journal을 만들거나 credential revision을 확인하지 않으며 저장 definition을 보존한다. Activation
command도 credential을 확인하지 않고 activation과 exact preference-clear transition만 public CAS
하나로 게시한다. External
connect, import, disconnect는 위의 operation별 journal 순서를 사용한다. 자유 형식 Provider
onboarding은 더 약한 경로를 빌리지 않고 아직 구현하지 않은 상태로 남는다.

repository가
없으면 빈 목록을 반환하고 상태를 만들지 않는다.
직접 history 읽기는
message-recovery interruption을 semantic record에 보존하며 discovery 불일치 진단은
stderr로 보낸다. Physical `v1` 형식만으로는 종료된 writer에 저장되지 않은 volatile
suffix가 있었는지 증명할 수 없으므로, 저장 history는 완전하다고 단정하지 않고
durability continuity를 `not-observable`로 기록한다. Chat은 간결하고 pipe 가능한
출력을 유지하며 기본 direct command는 이 continuity 경계를 stderr로 알린다. Transcript는
확인한 Journal cutoff, message-recovery 상태, durability-continuity 경계, discovery
consistency, 시간순 semantic record를 더한다. 파일 없음과 파일은 있지만 complete
envelope가 없는 상태도 서로 다른 direct-read failure로 유지한다. 어느 archived
출력도 backend를 시작하거나 이후 append를 구독하거나 저장소를 고치거나 그 자체로
continuation을 제공하지 않는다. live `yo --resume UUID`와 `yo --continue`는 대신
아래의 전용 typed continuation recovery를 사용한다.

## 실행 중인 observation view

선택한 TUI Projection은 표시만 바꾸며 Session authority를 바꾸지 않는다.

```text
읽기 전용 AgentPoll stream
    ├── Chat: 간결한 activity/message Projection + 편집 가능한 prompt
    └── 전체 semantic record Projection
          ├── Transcript: chronological command/event와 Activity detail
          └── RequestTrace: Journal 순서의 Session 전체 correlation record
                ├── 정확한 Chat/Transcript context → 선택적인 강조만 제공
                └── Request Audit → 명시적으로 사용 불가
```

현재 `input/view_binding.rs`의 F1/F2/F3가 Chat/Transcript/Request를
선택한다. 이 mapping은 typed 표시 정책 seam이며 Projection 상태가 아니다.
page·line 이동은 활성 view 자체의 viewport를 갱신하고, Chat과 Transcript는
각자의 context cursor도 보존한다. Request 이동은 완전한 diagnostic trace를
scroll하며 가까운 request 선택으로 내용을 바꾸지 않는다. view로 돌아오면
각 view가 보존한 상태를 복원한다.

세 mode 모두 session에서 pin한 appearance snapshot과 기존 Transcript
layout·Surface primitive를 쓴다. status 행은 활성 mode와 key를 표시하고,
좁은 frame에서는 `[C]123`, `[T]123`, `[R]123`으로 줄어든다. terminal
행이 하나뿐이어도 그릴 수 있다. Transcript와 Request는 full-page 읽기
전용 mode이므로 input 경로가 prompt editor에 도달하거나 submission을
만들지 않는다.

현재 TUI adapter는 semantic `TranscriptRecord`, typed durability 전환과 별도로
page를 나눈 payload-free `RequestTraceEntry` stream을 공개한다. Request stream은
각 correlation record의 `JournalSequence`를 보존하며 Journal lock, backend
payload, 물리 저장 형식을 노출하지 않고 같은 worker 변경 알림에서 끝까지
drain된다. Request Audit detail은 명시적으로 사용 불가 상태다. 현재 Chat이나
Transcript record에 정확한 `ActivityRequestRef`가 있으면 context로 표시할 수
있지만 Session 전체 trace 내용은 바꾸지 않는다.

## Durable Journal 조합 seam

실행 중인 `AgentSession`은 다음 local 조합을 사용한다.

```text
최초 SessionDescriptor (replay sequence 1, semantic cutoff 없음)
    ↓
명시적 JournalSequence를 가진 semantic Journal record
    ↓ runtime이 binding, accepted-request, outcome, Anchor correlation 추가
    ↓ codec/recovery가 완전한 correlation graph 검증
    ↓ 크기가 제한된 MessageSegment 구성
JournalCommit codec
    ↓ semantic commit 하나
JournalRepository
    ↓ durable semantic prefix와 검증
    ↓ physical append 하나
SessionRepository
    ↓ writer 시각 추가; payload와 완전한 discovery summary를 함께 checksum
Session-single-writer versioned JSONL physical v1

versioned JSONL
    ↓ 제한된 suffix 읽기 + semantic decode
Journal recovery
    ↓
RecoveredJournal 또는 명시적인 recovery 오류
    ↓ physical commit마다 binding epoch와 최신 완결 Anchor 도출

기존 repository root
    ↓ LocalSessionReader (생성·수리·writer lease 없음)
각 Session의 마지막 완결 envelope
    ↓ 닫힌 v1 shape와 CRC32C 검증
사용 가능한 discovery summary 또는 typed Session별 unavailable 결과
```

reader는 진단 문자열을 다시 해석하지 않고 격리, 손상, 미지원 schema, 완결 envelope
없음을 구분한다. 지원되는 summary에 Continuation Anchor가 없으면 `unavailable`,
미지원 schema면 `unknown`이다. 관찰한 pending marker는 storage를 만들지 않고 정확한
Session lease와 marker inode의 독립 append lock 양쪽에 대조한다. 그 marker lock까지
보유한 live owner만 append 이전 cutoff를 보일 수 있다. 후속 owner는 물려받은 marker를
입양할 수 없으며, 그 marker는 다른 Session을 숨기지 않은 채 해당 Session만 quarantine한다.
검사 도중 다른 append가 marker pathname을 교체하면 reader는 inode generation 변경을
감지하고 고정된 횟수 안에서 새 marker를 다시 검사하므로 잘못 quarantine하지 않는다.

Writer-capable repository는 혼합 버전 안전성을 위해 legacy root lock의 shared guard를
lifetime 동안 유지한다. Session을 load하거나 repair하기 전에 해당 Session의 exclusive
writer lease를 얻어 lifetime 동안 보유한다. 최종 repository-wide capacity 확인, marker
publish, append와 sync, 필요한 rollback, marker 제거만 짧은 root append coordinator
안에서 실행한다. Coordinator는 append 사이에 해제되며 lock·marker file은 record
capacity를 소비하지 않는다.

backend가 `CreateSession`을 받기 전에 worker는 UUIDv7 Session identity, Workspace
Host identity, 생성 Host의 canonical path bytes, UUID와 일치하는 시작 시각을 담은
descriptor-only incremental envelope 하나를 먼저 시도한다. descriptor는 Journal에
속한 탐색 데이터지만 frontend Transcript에 들어가거나 semantic `JournalSequence`를
소비하지 않는다. 첫 append가 storage pressure를 만나면 기존 gap 정책에 따라 이후
작업도 volatile하게 유지한다. 처음 성공하는 recovery snapshot은 descriptor로
시작하고 그동안의 complete semantic prefix를 함께 담는다.

replay sequence는 정규화한 모든 저장 record의 순서를 나타내고, `JournalSequence`는
command, event, backend correlation fact만 정렬한다. wire shape도 이 차이를 구조로
강제하므로 descriptor와 message record에는 `journal_sequence`를 넣을 수 없다.
recovery는 correlation record를 semantic 좌표로 indexing하고 모든 참조와 binding
transition을 검증한다. accepted request와 완료된 Turn이 durable prefix에서 모두
증명된 경우에만 Continuation Anchor를 공개한다.

live producer는 이제 `SessionCreated` 뒤에 최초 backend binding을 기록하고,
각 `SubmissionId`를 Start/Steer operation identity로 사용하며,
`TurnFinished(completed)`, resumable outcome, Continuation Anchor를 semantic commit
하나로 공개한다. provider adapter는 epoch나 Journal 좌표를 정하지 않고 opaque
evidence만 반환한다. runtime이 그 semantic identity를 소유하고 Journal만 sequence를
배정한다. User submission이 없는 내부 successor request에는 writer가 별도의 UUIDv4 identity를
부여한다. Transcript Projection은 correlation 전용 record를 제외한다.

Codex adapter는 model override를 보내지 않아 사용자의 effective model 선택을 보존하고,
`thread/start`가 반환한 `model`과 `modelProvider`를 기록한다. ephemeral thread가 아니라
저장되는 thread를 만든다. continuation에서는 versioned Codex locator만 decode하고
`thread/resume`을 정확히 한 번 보낸 뒤, runtime이 재개 상태를 공개하기 전에 반환된
thread·model provider·model identity를 최신 durable Anchor와 검증한다.

pending message text는 non-text 순서 경계 전에 immutable segment로 강제
저장되므로 동시 Activity event의 원래 순서를 보존할 수 있다. crash 뒤 열린
message가 남으면 recovery는 그 event를 버리지 않고 마지막 durable record 뒤에
interrupted seal을 제안한다. replay가 recovery
record를 합성해야 하거나, reopen 뒤 기존 durable prefix와 필요한 recovery
seal을 생략한 snapshot은 physical append 전에 거부한다. 자기 append 실패를
직접 관찰한 writer는 열린 모든 message가 실제 terminal seal을 받은 뒤 live-gap
snapshot 하나로 그 prefix를 완성할 수 있다. 그전까지 뒤따르는 record는 volatile
suffix에 남아 정상적인 snapshot 연기가 integrity 실패로 바뀌지 않는다. capacity
또는 storage-pressure 실패만 이 자동 재시도 경로에 들어간다.
integrity gap이나 예상하지 못한 snapshot gate는 현재 writer에서 memory-only로
남겨 증명할 수 없는 authority를 반복 제안하지 않으며, 이후 recovery owner가
repository에서 명시적으로 다시 구성해야 한다. 이는 구현된 failure
경계를 찾기 위한 설명이며, 동작 계약은
[Session Journal](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.observability.session-journal.md)과
[Session Repository](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.storage.session-repository.md)
KnowledgeUnit가 계속 소유한다.

CLI는 local repository를 기본으로 활성화한다. `YO_SESSION_REPOSITORY`로 root를,
`YO_SESSION_CAPACITY_BYTES`로 기본 1 GiB 상한을 바꿀 수 있다. Linux는 그 외에
`$XDG_STATE_HOME/yo/sessions` 또는 `$HOME/.local/state/yo/sessions`를 쓰고,
macOS는 `$HOME/Library/Application Support/yo/sessions`를 쓴다. `yo session`은
같은 root를 생성이나 writer lease 없이 연다. `yo --resume UUID`는 먼저 선택한
Session을 read-only로 검증하며, 직접 지정한 대상이 실행 불가능하면 저장소를
변경하지 않고 진단과 함께 archived Chat을 연다. `yo --continue`는 현재 Host와
정규화된 workspace에서 가장 최근 eligible Session을 고르고, 후보가 없으면 새
Session을 만들지 않고 실패한다. 실행 가능한 대상은 해당 Session writer lease 안에서
다시 검증하고 같은 Yo Session identity를 복원하며, 최신 durable Anchor 하나만
재개한다. 이전 Anchor로 fallback하지 않는다. remote storage, Request Audit
persistence, database나 compression backend, durable transport는 이 조합 밖에 남는다.

## 일시정지와 재개

`Ctrl+Z`는 application Session을 닫지 않고 터미널 소유권만 닫는다.

```text
Ctrl+Z press
    ↓
guard가 터미널을 복원한 뒤 TUI가 SuspendRequested 반환
    ↓
TerminationCoordinator가 활성 cleanup lease를 최종 확정
    ├── 종료 선택됨: 살아 있는 agent를 정리하고 해당 signal 재생
    └── 종료 없음: Idle로 반환
          ↓
yo-cli가 기본 SIGTSTP 동작을 적용하고 프로세스 정지
          ↓ SIGCONT
물려받은 SIGTSTP 상태 복원
          ↓
새 활성 lease와 터미널 소유 세대 시작
```

프로세스가 정지한 동안 `TuiSession`과 같은 agent 연결은 살아 있다.
터미널 input, raw mode, presenter, viewport 소유권, frame 이력은 남기지
않는다. 재개된 세대는 이 자원을 다시 획득하고 첫 화면 전체를 그린다.
보존된 appearance snapshot과 revision도 재진입 뒤 유지된다. 각 세대의 첫
redraw는 측정 전에 그 snapshot을 pin하고 완성된 `Surface`까지 그대로
운반한다. `process/job_control.rs`는 기본 `SIGTSTP` action을 임시로
설치하고, 재개된 뒤 물려받았던 action과 mask를 복원한다. process host는
resume 때 glyph profile을 다시 만들거나 선택하지 않는다.

`with_active_resource`가 종료 signal 없이 cleanup lease를 최종 확정한
뒤에만 프로세스를 일시정지할 수 있다. 이 경계에서 설정된 종료 signal이
도착하면 resource-cleanup callback이 보존된 agent를 정리하며, 일시정지
대신 바로 그 signal이 우선한다.

계약: [터미널 job-control 일시정지와 재개](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.job-control-suspend-resume.md)

## 종료와 정리

사용자 종료와 프로세스 종료는 프로세스 호스트가 signal 정책을 적용하기
전까지 같은 정리 경로를 사용한다.

```text
exit gesture 또는 typed TerminationEvent
    ↓
yo-tui loop가 종료 이유를 반환
    ↓
terminal guard가 표시 상태를 복원
    ↓
yo_tui::run_session_with_mode가 Exited 반환
    ↓
AgentSession::shutdown
  worker 중지 → backend 중지 → 활성 semantic work 종료
    ↓
TerminationCoordinator가 활성 resource lease를 마무리
    ├── 사용자 종료: yo-cli로 반환
    └── signal: 선택된 signal의 기본 disposition 적용
          ↓
일반 반환에서는 yo-cli가 설치했던 signal 상태를 복원
```

application Session이 끝날 때 TUI는 `UserRequested` 또는
`TerminationRequested`만 보고한다. 어떤 signal인지 식별하거나
프로세스의 마지막 동작을 선택하지 않는다. guard가 있는 runner는 어떤
결과든 반환하기 전에 터미널 상태를 복원한다. `run_agent_generation`은
터미널 연산이 실패했더라도 agent shutdown을 호출하고, 필요하면 두 실패를
모두 보고한다.

일반 반환에서는
[`TerminationCoordinator::shutdown`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/termination/mod.rs)이
설치했던 signal disposition과 설치 thread의 원래 mask를 복원한다.
종료 signal이 선택되면 `with_active_resource`는 TUI 정리 경로가
반환될 때까지 기다리고, 필요한 경우 보존된 agent도 정리한다. 그 뒤
signal을 일반 애플리케이션 오류로 바꾸지 않고 해당 signal의 기본
disposition을 적용한다.

## 첫 소유자 찾기

처음 실패한 경계에 가장 가까운 오류 문맥부터 따라간다.

| 보이는 문맥 | 시작 지점 |
|---|---|
| `starting Codex` | transport 시작을 포함한 `yo-core/backend/codex` |
| `creating the agent Session` | `yo-core/agent_session` 시작과 worker handshake |
| `terminal session` | `yo-tui/runner`와 터미널 mode 정리 |
| `agent cleanup` | `yo-core/agent_session::shutdown`, 그다음 runtime/backend 정리 |
| `process termination session` 또는 `process termination cleanup` | `yo-cli/process/termination` |
| `suspending the process` | `yo-cli/process/job_control` |

뒤이어 발생한 정리 실패를 버리지 않는다. 현재 최상위 경로는 서로
독립적인 정리 경계를 모두 시도하고 각 오류 문맥을 함께 보고한다.

## 계약 소유자

- [command와 event 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.command-event-boundary.md)
- [Session, Turn, Activity 의미](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.session-turn-activity.md)
- [활성 Turn input](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.active-turn-input.md)
- [Session Journal](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.observability.session-journal.md)
- [Session Repository](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.storage.session-repository.md)
- [Codex app-server adapter](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md)
- [typed TUI 흐름](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.typed-flow.md)
- [표시 mode 선택](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.mode-selection.md)
- [터미널 생명주기 복원](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.lifecycle-restoration.md)
- [Inline viewport publication](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.inline-viewport.md)
- [프로세스 종료 coordinator](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.process-termination-coordinator.md)
- [터미널 job-control 일시정지와 재개](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.job-control-suspend-resume.md)

실패한 경계를 찾았다면 [검증](../validation/)에서 수정 결과를
확인할 증거를 선택한다.
