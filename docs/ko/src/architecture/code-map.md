# 코드 지도

구체적인 타입이나 함수를 검색하기 전에 이 지도로 변경의 소유 경계를
고른다. 모든 소스 파일을 나열하지 않고 안정적인 책임과 진입점을
설명한다.

## 크레이트를 가로지르는 경로

프로세스 호스트는 provider와 frontend를 만든 뒤, frontend에 독립적인
세션 의미를 통해 둘을 연결한다.

```text
yo-cli main
├── yo-core CodexBackend
├── yo-cli TuiAgentConnection
└── yo-tui runner
        ↕ AgentConnection
    yo-core AgentSession
        ↕ bounded command lane + 합쳐지는 Journal 변경 lane
    worker-owned AgentRuntime
        ├── AgentEngine
        └── AgentBackend
```

현재 구현 경계는 다음과 같다.

- 프로세스 정책과 정리 순서는 `yo-cli`에 있다.
- Session, Turn, Activity, command, event의 의미는 `yo-core`에 있다.
- 터미널 상호작용과 화면 표시는 `yo-tui`에 있다.

승인된 책임과 향후 GUI에 대한 제약은
[프런트엔드 독립 코어 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.core.frontend-independent-boundary.md),
[모듈 및 호스트 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.architecture.module-boundaries.md),
[UI 전용 크레이트 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.crate.ui-only-boundary.md)가
계속 소유한다.

## yo-cli: 프로세스 호스트

| 경계 | 소유하는 책임 | 소유하지 않는 책임 |
|---|---|---|
| [`src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs) | 인자 해석, 터미널 획득 전 표시 방식과 glyph profile 선택, 작업 디렉터리 확보, provider 시작, 터미널 세대 재진입, 최상위 정리 결과 취합 | 에이전트 의미나 터미널 렌더링 |
| [`src/agent/mod.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/agent/mod.rs) | 구체적인 local Transcript cursor와 payload-free Request trace cursor를 포함해 `yo-core::AgentSession`을 TUI의 `AgentConnection` 포트에 맞게 연결 | provider 프로토콜 변환 또는 시기상조인 local·remote reader trait |
| [`src/command.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/command.rs), [`src/live.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/live.rs), [`src/session.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/session.rs), [`src/config.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/config.rs) | live startup, `yo default`, 명시적 `yo connect`, exact 또는 interactive `yo disconnect`, `yo session` 문법 분리, test 가능한 live 경계에서 `yo --resume UUID`와 현재 workspace의 `yo --continue` 선택, command-local 설정을 no-follow handle 하나로 안정적으로 capture하고 mutation 직전에 다시 확인, 구조화 모델 profile 필드에만 적용하는 scalar style 기반 숫자 분류, startup-only TUI frame rate와 Session 목록 날짜 설정, TTY 폭에 따른 열 우선순위, 저장 Chat·Transcript·Request와 typed discovery mismatch의 stdout/stderr routing | physical Session decode, semantic recovery, provider-native resume, 실행 중 설정 reload, 범용 반응형 plain-text layout |
| [`src/connection.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection.rs), [`src/connection/external.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection/external.rs), [`src/connection/disconnect.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection/disconnect.rs), [`src/connection/input.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection/input.rs), [`src/connection/input/file.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection/input/file.rs), [`src/connection/presentation.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection/presentation.rs) | 공유 recovery lane 하나에서 `yo default`, Local Codex connect, 정확히 설정된 external-model connect, target 하나의 managed disconnect 조율, prospective manual/managed 합성, 판단 정보를 먼저 두고 exact 보조 상세와 controlling-TTY 폭 줄바꿈을 제공하는 connect·disconnect preview, controlling-TTY target·확인·no-echo credential 입력, external connect의 명시적인 `--credential-file PATH --yes` 승인과 안정적인 current-user-only no-follow 파일 capture, 최종 config guard, 최초 성공 winner 보존, 저장 preference의 startup capture | 자유 형식 Provider onboarding, 추가 non-interactive secret channel, 범용 CLI widget framework 또는 repository의 physical storage |
| [`src/storage.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/storage.rs) | 사용자별 플랫폼 상태 루트와 별도 override 가능한 Session repository 루트 선택, local writer·생성하지 않는 reader·Host-identity-only 경로를 분리해 조합. live writer startup과 Local Codex 검증은 안정적인 Host identity 하나를 공유하고 read-only command는 기존 identity와 repository만 관찰 | Host identity의 의미나 physical Session record 의미 |
| [`src/process/job_control.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/job_control.rs) | 기본 `SIGTSTP` 동작 적용, 프로세스 일시정지, `SIGCONT` 뒤 물려받은 signal 상태 복원을 하나의 transaction으로 처리 | TUI 상태나 터미널 복원 |
| [`src/process/termination`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/termination/mod.rs) | Unix signal 설치, async-signal-safe readiness 연결, 관찰, 복원과 마지막 처리 | 터미널 상태 복원이나 frontend redraw 정책 |

프로세스 시작이나 종료가 실패하면 `main.rs`에서 오류 문맥에 표시된
소유자로 이동한다. signal coordinator는 `yo-cli`에 있다. TUI는 어떤
signal인지 알 필요가 없는 typed `TerminationEvent`만 받는다.
[프로세스 종료 coordinator](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.process-termination-coordinator.md)가
이 계약을 소유한다.

## yo-core: 에이전트 의미

[`src/lib.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/lib.rs)는
공개 facade다. GUI, TUI, provider adapter가 함께 쓸 새 기능이 필요할 때
여기서 시작한다.

| 모듈 | 소유하는 책임 | 다음 탐색 지점 |
|---|---|---|
| [`command.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/command.rs), [`event.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/event.rs), [`session.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/session.rs) | provider에 독립적인 command, 관찰 가능한 event와 outcome, typed identity, versioned `SessionDescriptor`. 릴리스 기준 Session identity는 저장소와 독립적인 UUIDv7이며 그 내부 시각은 descriptor 시작 시각과 일치한다 | 허용되는 상태 전이는 `engine` |
| [`host`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/host/mod.rs) | 불투명한 random UUIDv4 `WorkspaceHostId`, 원자적으로 만들고 권한을 제한한 local 사용자별 identity 파일, 생성 Host가 만든 lossless canonical workspace path 값 | Host identity가 일치할 때의 workspace 비교와 remote Host transport |
| [`workspace_reference`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/workspace_reference.rs) | frontend에 독립적인 workspace reference identity·provider port·revision 검색 메시지·Unicode 정규화 순위와 local 실행 provider의 background Git-ignore inventory | TUI 표시와 제출 시점 admission. `yo-cli`는 선택한 실행 provider만 조립 |
| [`skill_reference`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/skill_reference/mod.rs) | frontend에 독립적인 skill identity, 실행 환경 provenance, catalog generation과 entry revision selector, availability, revision-bound 검색 메시지 | TUI 표시와 제출 시점의 정확한 재검증. 구체적인 catalog adapter는 `backend` 아래에 남는다 |
| [`input`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/input/mod.rs) | 변경 불가능한 제출 text, 화면의 정확한 byte span에 묶인 순서 있는 typed reference occurrence, 안전한 reference token의 canonical Projection, UUIDv4 submission 연결, 제출 전체의 최종 outcome | queue와 worker 수락은 `agent_session`. 구체적인 reference admission은 다음 경계에 남는다 |
| [`model_service`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/mod.rs) | 안정적인 Provider·Account·Model identity, 명시적인 API dialect 선택과 정확히 하나의 built-in connector 파생, 정규화한 HTTPS endpoint, Provider·Account base profile과 whole-field model override를 완전한 typed profile 하나로 해석하는 경계, startup과 native resume이 공유하는 complete-binding 값 하나와 닫힌 durable decoder, structured profile 숫자를 위한 공유 YAML scalar-style guard, Provider·Account 범위 catalog·context-profile·credential resolution, provenance를 유지하는 manual/managed complete-binding composition과 typed `BindingConflict`, 주입하는 tokenizer-counting port, 원문을 감춘 resolved credential, 안전하고 크기가 제한된 local `credentials.yaml` private-revision exact-pair CAS, 크기가 제한되고 mode가 `0600`인 `connections.yaml` old-or-new CAS 안의 typed managed account와 complete binding, 닫힌 durable phase, candidate만 쓰는 multi-binding 검증, public-first disconnect 실행, 순수 exact-state recovery table을 가진 secret-free bounded `connection-operation.yaml` intent repository, operation lock 하나를 유지하며 journal·credential·public phase를 commit하는 same-directory local executor | connector wire 변환, command 수준 target·확인 표시 또는 설정 경로 선택은 process host |
| [`model_profile_admission.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_profile_admission.rs) | native runtime과 external connection 검증이 현재 실행할 수 있는 resolved profile field를 한 곳에서 admission | authored profile 해석, connector transport 또는 durable binding identity |
| [`model_connector`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_connector/mod.rs) | provider 중립 `openai-responses`와 `openai-chat-completions` request 직렬화, 같은 origin으로 제한한 bounded redirect, bearer 인증 HTTPS, 취소 가능한 request worker, dialect별 bounded SSE framing·correlation·finish/terminal 상태·usage | semantic Activity와 tool loop는 Yo-managed backend |
| [`tool`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/tool/mod.rs) | 안정적인 tool identity, 요청마다 고정한 registry, 닫힌 `yo.tool-schema/v1` dialect에 대한 크기 제한 인자 검증, argument와 output projection을 위한 주입형 semantic admission, 정규화한 approval binding, typed effect, 주입하는 단일 시도 execution-host port | 구체적인 운영체제 effect나 provider-hosted tool |
| [`engine`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/engine/mod.rs) | 결정론적인 Session, Turn, Activity, request 상태 전이 | 전이가 provider 경계도 지난다면 `runtime` |
| [`journal`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/journal/mod.rs), [`request_trace.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/request_trace.rs) | commit된 command, semantic event, 내부 backend correlation fact를 하나의 순서로 보존하는 live Projection, correlation 전용 record를 제외하는 sequence 기반의 제한된 Transcript 읽기, correlation 좌표를 보존하는 payload-free Request trace 읽기, live와 stored가 공유하는 Request Projection model, 동기식 durable publication, typed gap 상태, revision을 인식하는 크기 제한 `MessageSegment` 구성, recovery 검증. 실패한 semantic outcome은 message와 함께 명시적인 nullable code를 저장한다. 비공개 codec은 semantic `JournalSequence`와 physical replay 좌표를 분리하고 제한된 replay chain을 증분 검증하며 backend exchange, binding epoch, accepted request, resumable outcome, Continuation Anchor를 하나의 correlation graph로 검증한다 | capture 지점은 `runtime`, physical durability는 `session_repository` |
| [`session_repository`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/session_repository/mod.rs) | 저장 형식에 독립적인 append·replay·저장 Session 탐색/읽기 포트, snapshot 복구 gate, typed storage pressure, 첫 Session-single-writer versioned-JSONL 로컬 구현. 여러 process가 안정적인 root 하나를 함께 열고 서로 다른 Session을 동시에 쓸 수 있다. Writer-capable instance마다 legacy compatibility shared guard를 유지하고, Session 하나를 load하기 전에 exclusive lease를 얻으며, 최종 용량 확인과 physical append 동안에만 짧은 root coordinator를 얻는다. 현재 physical `v1` envelope는 checksummed discovery summary를 가진다. `LocalSessionReader`는 writer lease나 변경 없이 기존 저장소를 열고 Session마다 검증한 tail envelope 하나로 목록을 만들며, 존재 여부를 포함해 한 시점으로 고정된 history를 읽는다. `read_stored_session`은 파일 없음과 파일은 있지만 complete envelope가 없음을 구분하고, physical envelope와 semantic recovery를 검증하고 저장 전용 message segment를 semantic snapshot으로 합치며, message-recovery interruption, physical sequence를 포함한 최초 typed discovery mismatch, `v1`만으로는 종료 뒤 durability continuity를 관찰할 수 없다는 사실을 보존한다. 같은 검증된 recovery는 durable backend correlation fact 전체에서 frontend 독립적인 payload-free Request trace를 Journal 순서로 도출하며, physical envelope나 Request Audit payload는 노출하지 않는다. semantic recovery가 correlation chain을 증명한 뒤에만 binding epoch와 Continuation Anchor가 discovery에 들어간다. 저장 history를 읽을 때도 각 physical commit 지점에서 같은 상태를 다시 도출하여 summary가 빠졌거나 모순되면 거부한다. `read_stored_session_continuation`은 변경 없이 후보를 검증하고, `recover_stored_session_continuation`은 Session writer lease 안에서 같은 recovery를 반복한 뒤 descriptor, 최신 durable Anchor와 binding evidence, 복원할 semantic prefix와 frontend observation, 다음 Turn identity, 이미 승인한 Submission identity를 typed unit 하나로 반환한다. local `reader`와 `file` 모듈은 관찰과 변경 책임을 나눈다. `JournalRepository`는 candidate를 durable semantic prefix와 검증하고 semantic commit 하나를 physical append 하나와 조합 | provider-native resume, remote storage나 transport, Request Audit persistence, database나 compression 대안 |
| [`runtime`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/runtime/mod.rs) | backend 수락, semantic commit, Journal capture 순서, binding epoch와 SubmissionId 기반 operation identity 소유, provider 중립적인 binding/request/outcome evidence 검증, 완전한 continuation chain의 원자적 공개, codec으로 검증한 durable prefix에서 결정론적 Engine 복원, 재개한 backend identity 검증 뒤 full recovery snapshot 공개, 실패 시 활성 작업 종료 | provider port는 `backend/contract.rs` |
| [`agent_session`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/mod.rs) | frontend를 막지 않는 접근, 크기가 제한된 command lane, backpressure 동안 유지되는 submission identity, worker가 확정하는 수락 outcome, 용량 1의 Journal 변경 알림, 시작 취소, 종료 조율, 검증된 continuation에서 다음 Turn과 승인된 Submission identity를 복원하는 startup hydration | worker가 소유한 의미 처리는 `runtime` |
| [`backend/contract.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/contract.rs) | provider capability, command, semantic event, opaque binding/request/outcome evidence, polling, 취소, failure kind, 명시적 정리. evidence는 adapter fact만 가지며 epoch, operation ID, Journal 좌표를 정하지 않는다 | 구체적인 adapter |
| [`backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs) | `codex app-server` 생명주기, JSON transport와 protocol 분류, provider ID 연결, core event 변환, continuation evidence를 위한 backend/effective-model/thread identity 보존, ephemeral이 아닌 저장 thread, 최신 durable locator에 대한 검증된 `thread/resume` 한 번, worker가 소유하는 `skills/list` metadata catalog | 새 provider 동작을 노출하기 전 `backend/contract.rs`, structured dispatch 전 정확한 skill admission |
| [`backend/native`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/native/mod.rs) | connector-neutral observation을 사용하는 provider 중립 Yo-managed model loop, semantic model/tool Activity, 직렬 validation·admission·approval·execution 순서, request별 context admission, 제한된 model round, visible refusal replay, 응답별 정확한 binding·usage 귀속, cancellation 정리, 제한된 replay delta, 소진 시 completed non-resumable 처리와 binding latch | startup model 선택, tokenizer 구현, semantic-admission policy, 구체적인 local tool 구현 |

`AgentBackend`가 현재 provider 교체 지점이며 Codex wire value는
`backend/codex` 아래에 있다.
[command와 event 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.command-event-boundary.md)와
[Codex app-server adapter](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md)가
각 동작 제약을 소유한다.
[model-service binding](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.model.service-binding.md)과
[local account credential store](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.credentials.local-account-store.md)가
provider 중립 identity와 local secret 경계를 소유한다.
[OpenAI Responses connector](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.connector.openai-responses.md)와
[OpenAI Chat Completions connector](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.connector.openai-chat-completions.md)가
서로 다른 remote grammar를 소유한다. `backend/native`가 dialect에서 파생된 connector를 고정된 tool registry와
정확한 semantic replay에 조합하고 process host가 검증된 설정으로 그 backend를 선택해
조립한다. connector 자체는 semantic Activity를 소유하거나 tool을 실행하지 않는다.
[Session Journal](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.observability.session-journal.md)과
[Session Repository](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.storage.session-repository.md)가
durable replay와 storage 계약을 소유한다. 구현된 조합은 semantic commit을
encoding하고 message content를 크기가 제한된 `MessageSegment` record로
만든다. 권위 있는 replacement snapshot은 이미 durable한 segment를 바꾸지
않고 새 immutable message revision을 시작한다. non-text 순서 경계보다 먼저
pending text를 강제 저장하며, 같은 writer의 live gap snapshot과 reopen
recovery를 구분한다. 첫 릴리스 전 codec은 semantic commit `v1`만 쓰고 읽으며,
개발 중간 형식을 호환성 약속으로 남기지 않는다. frontend에 보이는 `JournalSequence`는 semantic cutoff만
나타내고 첫 descriptor와 normalized segment record는 비공개 replay 좌표로 정렬한다.
command, event, backend correlation record는 원래의 명시적 `JournalSequence`를 가지지만
descriptor와 정규화한 message record에는 그 좌표가 구조적으로 없다. recovery는 semantic
번호의 빈 구간은 허용하지만 중복·역행이나 이전 durable cutoff 안으로 들어온 incremental
번호는 거부한다.
descriptor는 replay sequence 1을 쓰지만 semantic `JournalSequence`를 만들지 않으므로
descriptor-only 첫 physical envelope에는 semantic cutoff가 없다.
`JournalRepository`는 JSONL을 append마다 다시 읽지 않고 복구한 상태에 새
suffix만 증분 검증한 뒤 local repository에 연결한다. 각 physical discovery
summary의 descriptor도 이 검증된 semantic prefix에서 만들고, 같은 recovery 상태에서
현재 binding epoch와 최신 완결 Continuation Anchor도 도출하여 envelope 값을 그대로 믿지
않는다. local writer는 같은 checksummed append 직전에 `updated_unix_millis`를 추가한다. 자기
storage-pressure 실패를 직접 관찰한 live writer는 snapshot 하나로 보존한
prefix를 완성할 수 있지만, reopen 뒤의 replacement snapshot은 그 prefix와
필요한 recovery seal도 보존해야 한다.

이제 실행 중인 `AgentSession` worker가 `JournalRepository` 호출 경로를
소유한다. CLI는 durable한 local Workspace Host identity를 확립하고 local
repository를 연 뒤, UTF-8을 lossy하게 바꾸지 않은 채 workspace를 canonicalize한다.
UUIDv7 시계 읽기 한 번으로 `SessionDescriptor`를 만든다.
`YO_SESSION_REPOSITORY`로 Session record 위치를 옮겨도 Host identity는 바뀌지
않는다. worker는 backend `CreateSession` 전에 descriptor를 첫 Journal envelope로
시도한다. 이 append를 할 수 없으면 descriptor와 semantic prefix를 complete
snapshot 하나로 함께 저장할 때까지 뒤의 activity도 memory-only로 남는다.
저장 형식에 독립적인 `StoredSessionReader`는 이제 제한된 discovery, typed
continuation eligibility, durable history replay를 제공한다. 미지원 schema는
`unknown`으로 계속 살펴볼 수 있고, 격리 상태와 Anchor가 없는 지원 record는
`unavailable`이다. `yo session`은 현재 workspace 목록, `--all` 탐색, full UUID
직접 지정의 읽기 전용 Chat·Transcript·Request 출력에 이 포트를 사용하지만 어떤 항목도 실행 가능하게 만들지는 않는다.

`yo-core`는 아직 `0.0.0` 내부 Pilot API다. 이 Slice는 persistent startup이 bare
`SessionId` 대신 완전한 `SessionDescriptor`를 요구하도록 의도적으로 바꾸고,
descriptor만 durable한 동안 `JournalDurability::Durable`에 semantic cutoff가 없을 수
있게 한다. `SessionId`는 이제 UUIDv7만 허용하므로 `as_uuid`는 UUID를 직접 반환하고,
허용된 identity에 대한 `SessionDescriptor::for_session`은 실패하지 않는다. 이는
compatibility shim을 추가하는 대신 계약을 바로잡는 의도적인
source-breaking 전환이다. public API가 확정되기 전까지 caller는 이 repository와 함께
migration해야 한다.

worker는 commit된 semantic 결과를 공개하기
전에 durable record를 쓴다.
streaming text는 크기·시간·ordering·종료 경계가 durable segment나 empty
revision의 `MessageReset`을 강제하기 전까지 process-local live revision으로 남는다.
명확히 append를 거부한 capacity나 storage-pressure가 발생하면 Session은 memory에서
계속 실행하면서 typed gap을 유지하고, 열린 message가 실제 terminal seal을 받은 뒤
complete snapshot이 성공하면 durability를 복구한다. 결과가 모호한 repository 실패는
현재 writer가 자동 재시도하지 않는 integrity gap으로 바뀔 수 있다. 공유
Transcript observation stream은 gap과 복구 전환을 그 영향을 받는 semantic record보다
먼저 순서대로 보존한다. CLI 연결은 이 typed observation을 전달하고 TUI 상태는 시각적 표현 정책을 선택하지
않은 채 최신 값을 보존한다. 저장된 Session 탐색, 읽기 전용 history, local
Codex-native resume은 연결되어 있다. resume에는 완결된 durable Continuation Anchor
하나, 같은 Workspace Host, 기록된 workspace, backend·model provider·model·thread의
정확한 반환 identity가 모두 필요하다. 이 증명이 없는 durability만으로는 Session을
실행할 수 없다.
local repository는 여러 live `yo` process가 같은 default root를 열고 서로 다른
Session을 쓰도록 허용한다. Session마다 한 process가 writer lease를 소유하고, 짧은
root append coordinator가 공유 capacity 확인을 정확하게 유지하되 append 사이에는
잠금을 유지하지 않는다. legacy root lock의 lifetime shared guard는 구·신 writer
binary가 겹칠 때 fail-closed한다.

backend adapter가 semantic `ModelWork`로 승인한 관찰 가능한 plan이나 reasoning
summary도 같은 durable message 경로를 따른다. yo가 받지 않은 숨겨진 model
reasoning과 승인하지 않은 backend-specific Request Audit payload는 이 semantic
경로 밖에 남는다.
remote storage, Request Audit persistence, database나 compression 선택, durable
transport는 이 경로의 범위 밖이다. `StoredSessionReader`는 Session 전용 read port이며
local·remote 공통 구현을 미리 주장하지 않는다. 실제 remote reader가 생길 때만
transport 공유 구조를 추출한다.

## yo-tui: 터미널 frontend

[`src/lib.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/lib.rs)는
실행 중인 session을 위한 runner facade를 좁게 유지하면서, 완성된 화면
상태와 터미널 연산, HTML Projection에 재사용할 타입을 공개한다.

| 모듈 | 소유하는 책임 | 다음 탐색 지점 |
|---|---|---|
| [`runner`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/mod.rs) | 실행 중인 session의 공개 facade, 터미널을 단독 소유하는 loop, 모든 live source의 필수 readiness, 무기한 idle 대기, 설정 가능한 120/60fps frame 합치기, 보존되는 Inline Chat publication cursor, 마지막 정리 결과 보고, 터미널에 독립적인 저장 Chat·Transcript·Request Projection | UI 의미 상태 전이와 후보 조율은 `runner/state.rs`, persistent 행 준비와 compact live size는 `runner/publication.rs`, frame-rate 정책은 `runner/frame.rs`, 저장 출력은 `runner/archival.rs`, 실행 중 조율·flush 후 geometry 관찰·보이는 motion scheduling은 `runner/unix.rs`·`runner/unix/presenter.rs` |
| [`runner/archival.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/archival.rs), [`runner/archival/request.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/archival/request.rs) | 읽기 전용 저장 Session 출력. Request는 정확한 관찰 경계, typed detail availability와 명시적인 Request Audit 미연결 상태를 포함해 payload-free correlation trace 전체를 durable Journal 순서로 그린다 | 저장 복구 또는 Request Audit 영속화 |
| [`appearance`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/appearance/mod.rs) | session이 소유하는 불변 appearance snapshot, 단조 증가 revision, resolved style role, 공개된 built-in Rich/ASCII glyph profile | 검증된 activity frame 순서·elapsed 기반 선택·최대 예약 marker 폭·연속 shimmer 계산·색상 깊이 해석·reduced motion은 `appearance/activity.rs`, profile 생성은 `runner/session.rs`, frame pinning은 `runner/state.rs` |
| [`plain`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/plain/mod.rs) | terminal cell 폭에 맞춰 고정 열을 유지하고 짧은 접힌 label/value pair는 폭 안에서 flow로 채우며 block 값은 독립된 한 줄을 사용하되 필요할 때만 label과 값을 분리하고, grapheme을 자르지 않고 개행한 뒤 필요하면 세로 card layout으로 전환하는 plain 목록 | 열의 의미와 접기 우선순위 또는 continuation hint, 설정, stdout TTY 정책, terminal 소유권 |
| [`input`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/input/mod.rs) | 해석이 끝난 semantic key event, 편집 buffer, 설정 가능한 binding, 종료 gesture, prompt 편집, typed view-switch 표시 정책, 사용 가능한 key action의 공용 terminal 표기 | terminal label만 다루는 곳은 `input/key_notation.rs`, 보이는 cursor 배치는 `prompt`, 선택한 Projection은 `runner/view.rs` |
| [`transcript`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/transcript/mod.rs) | 순서가 있는 사용자·에이전트 item, streaming revision, separator를 보존하는 범위 Projection, 대화 기록 layout, scroll 상태 | 단조 증가 publication cursor는 `runner/chat.rs`, prompt와 compact 조합하는 일은 `shell` |
| [`runner/view.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/view.rs) | Chat·Transcript·Request 선택, 상단 헤더가 없는 편집 가능한 Chat 화면, 읽기 전용 mode 헤더, 전체 Transcript Projection, 선택적인 정확한 context 강조를 포함한 Session 전체 payload-free Request trace, mode별 context·viewport 상태 | Journal 관찰과 editor dispatch는 `runner/state.rs`, 공통 layout·scroll은 `transcript` |
| [`prompt`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/prompt/mod.rs) | editor 내용과 cursor가 보이는 상태를 측정하고 그리며, 유효한 `@`·`$` token scan, 대체 query가 pending인 동안 마지막 usable panel 보존, stale provider update 거절, 선택 span 치환과 typed identity 보존, 보고된 scope에 따른 cached skill 후보 filtering | 편집 의미는 `input/editor`, 탐색은 실행 provider, freshness-gated 표시는 `overlay`, structured admission은 `yo-core` |
| [`overlay`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/overlay/mod.rs) | 검증된 선택 panel snapshot, 항목 availability와 독립적인 snapshot freshness, typed static/activity title status, enabled 항목 navigation·fitting, 선택적인 왼쪽 하단 filter 표시, 원자적인 `Surface` paint, token 범위의 단일 prompt-overlay slot | provider는 query·후보 filtering·preview와 accept된 제품 effect를 유지하고, routing·receipt는 `runner/state.rs`, 아래에 고정된 목적지는 `shell`이 소유한다 |
| [`shell`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/shell/mod.rs) | overflow를 typed 오류로 보고하며 Inline live 영역의 자연 높이를 측정하고 작업·prompt·metric·도움말 stack을 배분하며, 상태에 맞는 도움말을 원자적 우선순위 segment로 폭에 맞추고 pinned activity frame과 고정 문구 style sheen을 그린 뒤 완성된 frame의 cursor와 가장 짧은 보이는 motion demand를 보고 | 작업 행은 `shell/chrome.rs`, footer는 `shell/chrome/help.rs`, 표기는 `input/key_notation.rs`, cell 쓰기는 `surface`, host가 실제로 아는 status 값은 `runner/session.rs` |
| [`surface`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/surface/mod.rs), [`text`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/text/mod.rs) | adapter에 독립적인 cell 상태, Unicode grapheme과 너비, 경계가 있는 view, diff span, 터미널에 독립적인 text flow | Projection은 `terminal` 또는 `html` |
| [`terminal`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/mod.rs) | typed terminal operation과 ANSI encoding | 표시 정책은 `terminal/mode`, Unix effect는 `terminal/backend` |
| [`terminal/mode`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/mode/mod.rs), [`terminal/backend`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/backend/mod.rs) | 공유 transactional restoration, Inline·Fullscreen presenter, cursor 범위와 실제 scroll 증거를 가진 Inline typed-operation effect ledger, bounded write recovery, panic routing, crate-private direct unbuffered Unix 출력 경계 | operation/effect 순서와 exact correction은 `terminal/mode/inline/transaction.rs`, 정확한 downstream write와 flush 후 event는 `terminal/backend/unix`, 프로세스 signal 정책이 바뀔 때만 `yo-cli/process` |
| [`html`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/html/mod.rs) | 완성된 `Surface` 상태를 결정론적으로 브라우저에 Projection | 터미널과 브라우저 출력이 다르면 `surface` |

`runner::TuiSession`은 한 번의 터미널 소유 기간보다 오래 유지할 수 있는
간결한 Chat 대화 기록, editor, 대기 중인 요청, 세 observability view,
token 범위의 prompt-overlay slot 하나와 대기 중인 acceptance receipt,
backpressure로 전달되지 못한 agent dispatch 상태, 하나의 committed appearance
snapshot, bounded recovered-publication 증거를 소유한다. Chat은 편집 가능한 기본
mode다. 현재
Chat, Transcript, Request의 typed 표시 정책 binding은 각각 F1, F2, F3이며
Projection 상태는 이 key 선택을 소유하지 않는다. Transcript는 같은 읽기
전용 Journal 경로에서 받은 모든 committed command와 event를 그린다.
Request는 live Request-trace reader가 전달한 모든 bounded correlation record를
Journal 순서로 그린다. Chat이나 Transcript의 정확한 context는 선택적인 강조일
뿐 trace를 거르거나 가까운 record를 선택하지 않으며, Request Audit은 명시적으로
사용 불가 상태다. Transcript와 Request는
prompt를 대체하며 editor submission을 dispatch하지 않고 input을 소비한다.
각 view는 자체 context와 viewport 상태를 보존한다.
저장된 `yo session SESSION_ID --view request` 경로는 검증된 저장 Session 복구 뒤
같은 bounded record model을 Projection하며 context 강조를 제공하지 않는다.

Inline Chat은 완료되고 안정된 item의 최대 연속 prefix만 native terminal
history로 옮긴다. 준비 단계에서 후보를 이전 publication cursor, appearance
revision, terminal size, geometry epoch에 묶고, downstream write가 완료된
뒤에만 acknowledge한다. 아직 publication하지 않은 suffix, prompt, chrome,
overlay는 compact live `Surface`를 이룬다. Chat을 tail에서 떼어 검토하거나
읽기 전용 Transcript·Request를 보면 publication을 멈추고 terminal 전체 높이를
쓰며, Fullscreen은 항상 전체 semantic 상태를 그린다. flush가 성공하면 Unix
presenter가 대기 중인 resize 알림을 drain하고 terminal size를 새로 읽는다.
persistent prefix는 acknowledge하면서 stale live geometry는 버린 뒤 새 suffix
frame을 즉시 준비할 수 있다. terminal transaction이 output 오류를 exact하게
복구하면 controller는 correction 종류를 `TuiSession::publication_recovery_evidence`에
보존한다. Suspend는 semantic suffix를 출력하지 않고
cursor를 보존하며, 일반 종료와 typed termination은 아직 publication하지 않은
semantic 출력만 뒤에 붙인다.

승인된 view 의미는
[Chat, Transcript, Request Projection](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.observability.view-projections.md)
계약이 소유하고, prompt 주변 영역과 중단 affordance는
[정적 입력 chrome](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.chrome.input-stack.md)
계약이 소유한다. Panel 검증과 paint는
[selection overlay](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.overlay.selection-panel.md)
계약이, token 수명과 input 우선순위는
[prompt overlay routing](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.overlay.prompt-slot-routing.md)
계약이 소유한다.

현재 실행 중인 `AgentConnection`은 순서 있는 Transcript record, 별도의
durability transition, 각 record의 `JournalSequence`를 보존하는 payload-free
Request trace를 제공한다. Transcript adapter만 그 좌표를 버리고 Request
Audit detail도 제공하지 않으므로, view는
누락된 값을 추론하지 않고 이 제한을 드러낸다. 이 view layer는 Request
Audit을 persist하거나 또 다른 Journal owner를 만들지 않으며, worker가
소유한 repository 연결은 frontend 경계 아래에 남는다.

각 redraw는 측정 전에 appearance revision을 pin하고, paint와 완성된
`Surface`까지 같은 resolved snapshot을 사용한다. plain session output도
같은 session-owned 설정을 pin한다. runner는 터미널 소유 세대마다 하나의
elapsed sample을 전달한다. appearance는 이 값으로 marker frame을 직접 선택하고
reduced motion에서는 첫 frame을 고정하며, 검증된 frame 가운데 가장 넓은 폭을
고정 marker 영역으로 유지해 frame 변경이 문구 위치나 fitting을 바꾸지 않게 한다.
보이는 animated marker나 activity 문구 sheen은 한
grapheme pulse도 포함해 period를 반환하지만 static·숨김·빈 값·reduced-motion
indicator는 motion demand를 만들지 않는다. 완성된 frame은 보이는 indicator 가운데 가장 짧은 양수 period를 보고한다.
`runner/unix.rs`는 다음 epoch 경계를 계산하고 놓친 tick을 건너뛴다.
`runner/frame.rs`는 motion 요청이 due가 되면 input·background 변경과 같은
readiness 기반 120/60fps frame 경계에 합친다. presenter와
HTML은 계속 완성된 `Surface`만 소비한다. 모든 public `TuiSession` 생성자와
one-shot runner는 appearance를 publish하기 전에 process host가
TrueColor·Limited·Unknown 중 하나와 Standard·Reduced motion preference 중
하나를 명시하도록 요구한다. `TuiSession::new`는 기본 Rich glyph를 선택하고,
`TuiSession::with_glyph_profile`은 mutable theme state를 노출하지 않은 채
host가 built-in ASCII profile도 선택하게 한다. `TuiSession::with_session_info`는
같은 명시적 publication 경계에 backend와 workspace label을 더한다.
`TuiSession::with_frame_rate_limit`은 기본 120fps frame 합치기 정책을 유지하거나
semantic 상태 전이를 바꾸지 않고 host가 60fps로 낮추게 한다. CLI는 startup-only
`tui.max_fps` 설정을 이 정책으로 옮긴다. 향후 GUI는 source readiness를 재사용하되
자체 event loop와 redraw/vsync 정책을 유지할 수 있다.
chrome은 알 수 없는 model, context, Git, permission 값을
만들어내지 않고 생략한다.
보존된 상태에는 해당 agent Session의 식별자가 있으므로 재진입할 때도 같은
agent 연결을 유지한다.
`runner/unix.rs`는 매 터미널 소유 기간마다 터미널 입력, presenter,
viewport 소유권, frame 이력을 새로 얻으며, 이 자원들은 `TuiSession`으로
옮기지 않는다. 정리가 성공한 `Ctrl+Z`는 이 세대 전용 자원을 모두 복원한
뒤에만 `TerminalOutcome::SuspendRequested`를 반환한다.

Appearance 계약:
[session publication](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.session-publication.md),
[frame 일관성](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.frame-consistency.md),
[glyph profile](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.glyph-profiles.md),
[activity motion profile](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.activity-motion-profile.md),
[activity motion scheduling](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.activity-motion-scheduling.md),
그리고
[resolved cell style](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.resolved-style.md).

Runtime scheduling 계약:
[bounded frame scheduling](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.frame-scheduling.md)과
[공정한 readiness 기반 event source](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.event-source-scheduling.md).

Inline publication과 compact live geometry는
[Inline viewport 계약](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.inline-viewport.md)이 소유한다.

`surface`는 공통으로 완성된 상태다. 터미널과 HTML Projection은 이를
각자 소비하며, 어느 쪽도 다른 쪽의 layout 의미를 정의하지 않는다.

## 저장소 개발 도구

[`tools/xtask`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/xtask/src/lib.rs)는
`yo` 제품이 아니라 이 저장소를 관리하는 구조화된 검사를 소유한다. 변경
경로와 commit trailer를 분류하여 Slice 검수와 Developer Docs 영향을
확인하고, Rust test에 이해 가능한 인접 설명이 있는지도 검사한다.
`activation_slice` 모듈은 작은 semantic request를 받아 현재 `develop`을
고정하고 canonical Methexis activation contract를 발행하며, Direct Slice
worktree를 생성해 둘을 bind하고 exact 부분 setup을 복구한다. review-packet
모듈은 일반 후보에는 active ContextBuild를 쓰고, 이후 activation request 하나에는
명시적으로 versioning한 prospective operation을 쓴다. 후자는 activation을 허가하지
않고 제안된 Checkpoint와 active-record 전환을 결속한다. bootstrap 모듈은 trusted
`develop`의 정확한 versioned capability를 요구하고 닫힌 4개 경로 activation 전환만
허용하므로 구현과 workflow 변경은 계속 일반 검수 경로를 쓴다. `slice_close`
모듈은 수용 commit, Slice patch, 검수 증거, binding, ref,
깨끗한 worktree가 모두 일치한 뒤에만 hash-addressed 로컬 정리 plan을 만들고
적용한다. 그 storage 경계는 안전하지 않은 plan file 입력을 거절한다.
`hk.pkl`은 검사를 언제 실행할지 결정하고, `xtask`는 규칙의 구현과 test를
담당한다. Methexis와 Librarian은 각자의 지식 domain 책임을 유지하며,
단순한 외부 명령 조합은 `hk` 또는 작은 검증 script에 남는다.

소유자를 골랐다면 [검증](../validation/)을 변경 경계에서 증거로
이어지는 단일 지도로 사용한다. 실제 터미널 동작이 관련되면
[터미널 환경 매트릭스](../validation/terminal-matrix.md)를 따른다.
[Slice](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#slice-contract)를
닫기 전에는 변경이 실제로 지나간 경계에 대해서만 검사를 넓힌다.
