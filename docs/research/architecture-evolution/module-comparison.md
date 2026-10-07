# Pi·Codex·Yo 모듈 설계 대조

> Status: non-authoritative architecture proposal; 구현·계약 변경 아님
>
> 기준: 2026-10-05. Yo `880467b3186ac7ace0111acd37cb1aa334c9dc4c`,
> Pi `v1.0.2`, Codex `rust-v0.160.0`; [원본 고정 정보](./evidence.md).

비교 단위는 기능 유무가 아니라 생성 경계, 의존성 방향, 공개 API, mutable state의
소유자, 수명과 변경 파급이다. 아래 판단은 고정된 소스의 정적 검토 결과이며
성능 측정이나 모든 실행 경로의 검증 결과가 아니다. 기존 [레이어별 검토](./layer-review.md)를
보완하며, 각 프로젝트의 같은 이름이 같은 책임을 뜻한다고 가정하지 않는다.

Yo는 실행 의미와 backend/connector 분리를 유지하는 편이 좋다. Pi에서 가져올 핵심은
workspace별 준비와 Session 생성의 분리이고, Codex에서는 서비스·클라이언트·실행 수명의
분리다. 우선 바꿀 것은 CLI의 설정·준비 결과에 섞인 TUI 의존성과 managed 내부의 넓은
상태 접근이다. Crate 수를 늘리는 것 자체는 개선 기준이 아니다.

## 1. 조립, 설정, 실행 수명

| 축 | Pi 실제 코드 | Codex 실제 코드 | Yo 현재 → 판단 |
|---|---|---|---|
| 실행 준비 | `createAgentSessionServices`가 cwd별 ModelRuntime/SettingsManager/ResourceLoader와 진단을 구성하고 `createAgentSessionFromServices`가 SDK에 전달 | `InProcessStartArgs`, `in_process::start`, `bootstrap::configure`로 입력·정책 준비·runtime 생성을 구분 | `prepare_agent`는 picker/TTY/print와 실행 조립을 함께 처리. 실행 설정 snapshot과 중립 launch 결과부터 분리 |
| Session 생성과 교체 | SDK가 Agent/AgentSession을 조립하고 AgentSessionRuntime이 Session+services를 소유 | InProcessClientHandle이 task/event receiver 수명과 consuming shutdown 소유 | core AgentSession이 worker를 소유하나 TuiAgentConnection 생성 경로가 입구. 중립 factory를 만들고 TUI wrapper는 이후 구성 |
| mutable 설정 | services는 concrete 객체를 공개하고 SDK에 global stream 설정도 있음 | bootstrap이 effective policy/auth를 조립; embedded와 standalone이 공통 processor 사용 | Config의 model/tool/skill 설정과 Theme/PromptTemplates/clipboard를 분리. live Session에 mutable 전체 Config를 공유하지 않음 |
| 교체 실패 | AgentSessionRuntime은 기존 runtime teardown 후 새 runtime 생성 | 소유 handle과 명시적인 shutdown 경계가 있음 | Yo의 후보 준비 실패 시 기존 Session 보존 규칙 유지. Pi의 교체 순서를 복제하지 않음 |

근거: Pi [services factory](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/agent-session-services.ts),
[SDK](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/sdk.ts),
[runtime](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/agent-session-runtime.ts);
Codex [in-process](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/app-server/src/in_process.rs),
[bootstrap](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/app-server/src/in_process_bootstrap.rs);
Yo [Config](../../../crates/yo-cli/src/state/config.rs),
[PreparedAgent](../../../crates/yo-cli/src/application/runtime/startup/model.rs),
[prepare](../../../crates/yo-cli/src/application/runtime/startup/prepare.rs),
[print](../../../crates/yo-cli/src/application/runtime/print.rs).
Print가 LiveOptions를 만들고 TuiAgentConnection을 다시 AgentSession으로 벗기는 것이
현재 결합의 구체적인 사례다. 공유 factory 입력에 LiveOptions/AppError/Theme를 옮겨서는 해결되지 않는다.

## 2. Frontend, client, service, protocol

| 축 | Pi | Codex | Yo 현재 → 판단 |
|---|---|---|---|
| 소비자 경계 | interactive와 RPC가 같은 AgentSessionRuntime을 별도 mode로 감쌈 | AppServerClient의 InProcess/Remote와 별도 request handle; TUI가 client/protocol 소비 | 기존 설계의 service/protocol/server에 consumer client owner가 빠짐. 실제 remote 소비자 도입 때 `host::client` 추가 |
| pending 명령 | RpcClient가 request IDs/pending promises/timeout/process-exit 실패 소유 | client command/response channel은 client 내부 | TUI PendingDispatch는 core PendingCommand alias. AgentCommand를 비공개 필드로 감싼 local admission token을 remote 재시도 API로 사용하지 않음 |
| 관찰 | JSON event adapter가 core events를 투영하고 누적 stream snapshot 제거 | client AppServerEvent가 notification/request/lag/disconnect 구분 | AgentPoll은 TuiDocument/TuiStatusLine/LinkResolver까지 포함. 공용 bounded projection envelope를 별도로 두고 UI 타입은 adapter에 유지 |
| 연결 수명 | RpcClient.stop은 자신이 띄운 child 종료 | runtime owner와 client/event handles 구분 | 현재 TuiAgentConnection은 AgentSession을 소유하고 Session drop은 shutdown. service Session entry와 attachment를 분리해야 disconnect 후 유지 가능 |
| 확장 UI | extension 타입이 pi-tui와 interactive Theme를 import; RPC의 custom/widget 일부는 지원하지 않음 | TUI가 renderer를 소유하나 legacy_core escape hatch도 남음 | Pi의 terminal-shaped extension port와 Codex legacy 우회 경로를 공용 API로 복제하지 않음 |
| 전송 비용 | mode별 adapter와 RPC correlation | local도 JSON-RPC result envelope 사용; client event queue는 unbounded | Yo direct call의 무직렬화·bounded queues 보존. Codex의 API 분리를 채택하되 버퍼 정책은 별도 |

근거: Pi [interactive](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/interactive/interactive-mode.ts),
[RPC mode](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/rpc/rpc-mode.ts),
[RPC client](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/rpc/rpc-client.ts),
[extension types](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/extensions/types.ts);
Codex [client](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/app-server-client/src/lib.rs),
[TUI session](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/app_server_session.rs);
Yo [AgentConnection/AgentPoll](../../../crates/yo-tui/src/runner/agent.rs),
[PendingCommand](../../../crates/yo-core/src/agent_session/contract.rs),
[Journal→TUI bridge](../../../crates/yo-cli/src/application/agent/journal.rs),
[Session Drop](../../../crates/yo-core/src/agent_session/lifecycle.rs).

Yo의 bridge는 observation sequence를 내부에서 소비하고 bare AgentPoll을 반환하며 trace는
별도 cursor로 읽는다. 이것을 그대로 wire event로 보내면 reconnect용 ordered suffix가 되지 않는다.
Service projection이 좌표·gap 의미를 소유하고 frontend가 표시 형태를 만든다. Local pending
admission과 remote 전송 후 결과 불명은 다른 상태다. 후자는 operation inspection 대상이며
새 명령 자동 재전송의 근거가 아니다. Session 종료, 연결 끊김, controller 상실, resync도 구분한다.

## 3. Agent state, tools, context, persistence

| 축 | Pi | Codex | Yo 현재 → 판단 |
|---|---|---|---|
| 실행 분할 | Agent, agent-loop, 제품 AgentSession, SessionManager | CodexThread 밖에 private Session/SessionState/TurnState/StepContext/ContextManager | AgentSession→worker Runtime→Engine/backend 유지. managed의 round/batch/context를 private 상태 소유자로 강화 |
| 결과 게시 | 병렬 loop가 Promise.all 결과를 source order로 반환; AgentSession message_end가 저장 | ToolCallRuntime은 envelope 반환; turn loop가 FuturesOrdered를 drain하며 Session에 기록 | managed는 BackendEvent 제안, Runtime이 검증·Journal 게시. scheduler가 committed 결과를 소유한다는 후보 API 수정 |
| 상태 접근 | AgentSession이 agent/sessionManager/settingsManager 공개 | 내부 타입은 pub(crate)/pub(super)이나 ToolCallRuntime에 Arc<Session>, 넓은 SessionServices 전달 | 전체 NativeModelBackend/HostContext 공유보다 좁은 transition 메서드. upstream의 큰 Session/service bag은 복제하지 않음 |
| context 복원 | Agent 메시지와 SessionManager 기록 공존; 요청 때 저장 projection 재구성 | SessionState가 ContextManager 소유 | core가 Journal에서 검증·복원한 replay projection과 managed 작업용 replay 역할 명시. 중복처럼 보인다는 이유로 합치지 않음 |
| I/O와 lock | event listeners를 await하며 persistence 연계 | state mutex와 settings persistence semaphore 분리 | worker commands/events가 admission state lock 중 runtime 실행·poll. 현재 backpressure 계약을 유지하면서 새 host/memory/budget wait 전 lock 범위를 재설계 |

근거: Pi [agent loop](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/agent/src/agent-loop.ts),
[AgentSession](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/agent-session.ts),
[SessionManager](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/session-manager.ts);
Codex [tool runtime](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/tools/parallel.rs),
[turn loop](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/session/turn.rs),
[Session](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/session/session.rs),
[SessionState](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/state/session.rs);
Yo [managed state](../../../crates/backends/managed/src/backend.rs),
[Runtime publication](../../../crates/yo-core/src/runtime/poll.rs),
[worker commands](../../../crates/yo-core/src/agent_session/worker/commands.rs),
[worker events](../../../crates/yo-core/src/agent_session/worker/events.rs).

Rust에서는 자식 모듈이 부모의 private field에 접근할 수 있다. 현재 managed처럼 부모에
모든 상태를 두고 여러 파일에서 impl만 나누면 소유권은 분리되지 않는다. 다음은 분리 후보이며
타입 이름은 아직 확정하지 않았다. 각 기능에 필요한 범위에서 순차적으로 적용하고,
대규모 선행 리팩터링을 모든 기능의 시작 조건으로 삼지 않는다.

```text
backend orchestration
  turn::TurnExecution         turn identity/phase/steer
  response::RoundAccumulator  streamed response/tool calls
  tools::ToolBatch             source-index slot enum; indices/queues are derived
  context::ContextWindow      working replay/retained groups
  context::CompactionMachine  private prepared checkpoint
  request::RequestAdmission  immutable request + accounting
```

상태 타입은 자식 모듈 안에 정의하고 field는 private으로, 필요한 전이 메서드만 pub(super)로 둔다.
ToolBatch의 physical readiness→ordered proposal→Runtime publication→retire를 구분한다.
기존 event drain 후 다음 poll로 넘어가는 barrier를 먼저 명문화하고, 독립 poller를 도입할 때만
exact generation/source-index acknowledgement를 추가한다. Journal writer는 전달하지 않는다.

Ordinary, active/idle compaction, protected request의 dispatch는 현재 별도 경로에 있다.
[request](../../../crates/backends/managed/src/backend/request.rs),
[active compaction](../../../crates/backends/managed/src/backend/compaction/active.rs),
[idle compaction](../../../crates/backends/managed/src/backend/compaction/idle.rs),
[protected request](../../../crates/backends/managed/src/backend/secret.rs)를 공통 private admission/dispatch 경계로 모은다.
Goal hard budget은 모든 대상 경로의 dispatch 전 permit에 연결한다. dispatch 후 발생하는
ModelRequestAccepted를 사전 허가로 재사용하지 않는다. Worker-owned GoalLedger와 parent-funded child의
bounded 요청/응답을 분리하여 backend가 같은 worker를 동기 callback으로 기다리지 않게 한다.
Protected receipt barrier도 유지한다.

Journal의 내부 의존성에는 [durable](../../../crates/yo-core/src/journal/durable.rs)
→[repository adapter](../../../crates/yo-core/src/session_repository/journal.rs)
→journal codec으로 돌아오는 경로가 있다. Schema 확장으로 해당 부분을 수정할 때 semantic codec adapter를
journal 쪽으로 모으고 physical append/local I/O는 repository에 남기는 작은 정리를 검토한다.
새로운 범용 log crate는 필요하지 않다. 지금 이동을 확정하거나 결함으로 판정하는 것은 아니며,
기존 public reexport와 continuation 복원의 호환성을 확인한 뒤 판단한다.

## 4. Model/provider, 확장, 장기 기억의 경계

| 축 | Pi | Codex | Yo에 적용할 판단 |
|---|---|---|---|
| provider라는 이름의 의미 | ModelRuntime이 catalog/auth/config/virtual routing/stream을 묶고 ProviderConfigInput이 streamSimple 등을 주입 | ModelProvider가 account/auth recovery, capability, memory extraction용 model 선택 등도 담당 | Yo provider는 catalog/discovery, connector는 wire, backend는 loop를 소유. 이름이 대응한다는 이유로 하나로 통합하지 않음 |
| 확장 등록 | registerProvider/recomposeProvider와 snapshot refresh로 실행 시 구성을 변경 | model-provider/config/transport 등의 여러 crate를 조합 | Yo frozen registry·complete binding·exact ConnectorId+dialect factory 유지. 새 dialect는 connector, catalog 차이는 provider라는 변경 축을 보존 |
| 리소스 공급 | ResourceLoader가 instructions/skills/themes 등을 묶음 | ContextManager와 SessionServices 등이 분담 | instruction discovery는 host, admitted bytes는 core/managed 소유. UI theme을 같은 port에 넣지 않음 |
| memory의 새 API | agent-core는 준비된 context를 소비하고 제품 리소스는 coding-agent 쪽에서 담당 | provider가 memory model도 선택하지만 Yo memory store 소유권의 근거가 되지는 않음 | CRUD/search/CAS/forget repository는 host 소유. core에는 immutable admitted snapshot/provenance/revision과 필요한 최소 admission capability만 둠 |

근거: Pi [ModelRuntime](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/model-runtime.ts),
[provider composer](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/provider-composer.ts),
[ResourceLoader](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/resource-loader.ts);
Codex [ModelProvider](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/model-provider/src/provider.rs),
[crate dependencies](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/model-provider/Cargo.toml);
Yo [exact factory](../../../crates/yo-cli/src/execution/model/native.rs),
[binding](../../../crates/yo-core/src/model_service/binding.rs),
[connector port](../../../crates/yo-core/src/model_connector/port.rs),
[provider](../../../crates/providers/openrouter/src/lib.rs),
[wire implementation](../../../crates/connectors/openai-responses/src/lib.rs).

Memory off/forget과 input commit의 경합은 host store의 bounded admission guard로 처리한다.
Worker가 직접 guard를 얻어 자신의 snapshot commit까지 유지하고, host가 lock을 잡은 채
worker queue를 기다리는 구조는 피한다. Parent budget 등 다른 writer를 기다리는 작업은 guard를 얻기 전에 마친다.
이를 기존 core local store 전체를 옮기는 이유로 삼지 않는다. 새 공개 API부터 semantic values와
concrete repositories를 구분하고 ToolBatch/GoalLedger/store lock은 공개하지 않는다.

### 승인·sandbox·도구 확장의 모듈 경계

| 축 | Pi | Codex | Yo 결정 |
|---|---|---|---|
| 도구 확장 adapter | ToolDefinition을 AgentTool로 감싸고 제품 ExtensionToolContext를 factory로 공급 | ToolRuntime/Approvable/Sandboxable과 ToolOrchestrator가 실행·승인·sandbox를 연결 | core의 tool 실행 port와 host adapter를 유지. renderer/전체 Session 객체를 tool port에 넣지 않음 |
| 승인 재사용 | agent-loop의 beforeToolCall hook에서 차단/허용 결과를 받음 | ApprovalStore와 with_cached_approval이 session 결정을 보관 | host permission store의 scoped grant와 매 호출 exact authorization을 분리. generic serialized cache key를 권한 identity로 복제하지 않음 |
| 병렬 실행과 승인 대기 | executeToolCallsParallel이 prepareToolCall을 순서대로 await한 뒤 실행 함수를 Promise.all로 시작 | parallel runtime이 shared/exclusive lock을 잡고 handler dispatch를 await하며 그 안에서 approval 경로에 진입할 수 있음 | waiting approval은 running slot/실행 lease를 잡지 않음. 앞선 reserved claim의 충돌 순서는 별도로 유지 |
| 격리 책임 | 위 hook/adapter만으로 OS confinement를 증명하지 못함 | orchestrator의 정책 결정과 SandboxManager의 실행 변환을 구분 | scheduler는 readiness/claims, permission adapter는 authorize/revalidate, sandbox adapter는 실제 제한 집행을 소유 |

근거: Pi [tool wrapper](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/tools/tool-definition-wrapper.ts),
[prepareToolCall/parallel loop](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/agent/src/agent-loop.ts);
Codex [orchestrator](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/tools/orchestrator.rs),
[approval/sandbox traits](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/tools/sandboxing.rs),
[parallel gate](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/tools/parallel.rs);
Yo [exact approval](../../../crates/yo-core/src/tool/approval.rs),
[execution port](../../../crates/yo-core/src/tool/execution.rs),
[tool admission](../../../crates/yo-cli/src/execution/tools/admission.rs).

Pi hook가 오래 대기하면 parallel 함수의 실행 시작도 늦어진다는 것은 이 호출 순서에서의 추론이다.
실제 Pi 승인 UI 성능을 측정한 결과는 아니다. Codex 역시 병렬 잠금 범위를 그대로 가져오면
Yo가 해결하려는 승인 대기 병목이 남을 수 있다. Yo에서는 독립 read가 승인 대기와 겹쳐
실행되도록 하되, opaque shell의 workspace-exclusive claim까지 건너뛰지는 않는다.

## 5. 현재 의존성 방향과 목표 내부 구조

다음은 Cargo production path dependencies의 요약이며 실행 시 call graph가 아니다.
같은 범주의 crate를 묶었다. Managed의 Kimi connector 의존성은 dev-dependency이므로 제외한다.

```text
yo-cli -> yo-tui, yo-core, concrete backends/connectors/providers, yo-yaml
yo-tui -> yo-core
yo-core -> backend foundation, yo-yaml
managed / delegated backends -> backend foundation, yo-core
concrete connectors -> connector transport, yo-core
connector transport -> yo-core
concrete providers -> yo-core
```

Connector transport는 core의 ConnectorError/ModelConnectorEvent 타입을 사용한다.
따라서 core에 의존하지 않는 순수 byte crate는 아니지만, dialect별 wire/replay policy를
포함할 이유도 없다. Foundation/core에서 concrete 구현으로 향하는 역방향 의존성은 추가하지 않는다.

독립 GUI/web 소비자가 들어오는 단계의 host 내부 구조 후보는 다음과 같다.

```text
host bootstrap -> assembly (private factory wiring)
               -> service (factory를 입력으로 받음)
service -> sessions / attachments / admission / projections / artifacts
client::local -> service API
client::remote -> protocol mapping + optional transport
server -> protocol mapping -> service API
assembly -> core + concrete backend/connector/provider + adapters
adapters -> 필요한 core ports만
```

Service는 주입된 좁은 factory capability를 사용하고 assembly 구현을 import하지 않는다.
Factory는 session map/controller를 모르고 service는 concrete connector 선택을 모른다.
Tasks는 lifecycle owner에게 bounded creation intent를 보내며 session map을 중복 소유하지 않는다.
`tasks↔service`가 전체 상태를 상호 참조하는 구조, `service→wire DTO`, `adapters→service/server`,
`host→TUI/CLI` 의존성은 금지한다. 전체 mutable HostContext나 범용 service locator는 도입하지 않는다.

공개 API는 ExecutionSettingsSnapshot, SessionLaunch, 준비 진단, host-owner shutdown handle,
attachment/request/read handle, bounded immutable projection, typed errors로 제한한다.
Factory만 있는 단계에서는 CLI가 중립 Session을 받는다. Service 도입 후에는 service가 Session을
소유하며 client에게 raw mutable Session을 함께 전달하지 않는다. Session map, pending queue, grant
store, concrete backend, wire dispatch table은 private으로 둔다. 모듈마다 trait를 만들 필요는 없다.

Protocol DTO/schema, WS/HTTP, web assets는 선택적 build surface로 두어 direct consumer에
web stack을 강제하지 않는다. Codex의 client crate도 app-server/core에 의존하므로 별도 crate가
곧 가벼운 의존성을 뜻하지는 않는다. 먼저 하나의 host crate 안에서 module/features로 나누고,
독립 remote-only 배포 소비자가 요구할 때만 얇은 client/protocol crate 분리를 판단한다.

## 6. Yo의 이동·분할·유지 맵

| 현재 위치 | 최소 변경 | 남기는 책임 |
|---|---|---|
| CLI state/config + startup/model,prepare | immutable execution snapshot / SessionLaunch / neutral result로 분할 | theme/glyph/picker/prompt restoration/error 표시는 CLI/TUI |
| application/agent/lifecycle | neutral Session factory를 먼저 CLI 내부에 추출 | signal→cancel 변환과 TUI wrapper는 CLI |
| execution/model/native, backend factory | 독립 소비자 도입 시 host private assembly로 이동 | complete identity에 따른 exact 선택, UI label은 frontend |
| execution/tools filesystem/command/registry/admission | host tool adapters로 이동 | command child의 spawn/wait/reap은 adapter 내부 |
| execution/process termination/job_control/external_editor | CLI에 유지 | executable별 signal/TTY/editor 정책 |
| initialize_process_file_mode | 파일 위치와 관계없이 bootstrap이 호출 | process-wide effect를 library 시작의 숨은 부수 효과로 만들지 않음 |
| application/agent connection/journal/termination | TUI adapter로 유지하고 client에 연결 | terminal projection, local pending token 변환 |
| execution/image/host | input evidence/admission과 clipboard 취득을 분리 | clipboard는 frontend, host는 검증된 입력을 소비 |
| managed backend state/request | 자식 module의 private 상태와 전이로 단계적 이동 | core sole Journal writer, exact replay, protected request barrier |
| core repositories/credentials/reference ports | 현재 owner 유지 | 공유 코드 또는 I/O라는 이유만으로 host에 옮기지 않음 |

원본 코드: [agent lifecycle](../../../crates/yo-cli/src/application/agent/lifecycle.rs),
[process](../../../crates/yo-cli/src/execution/process.rs),
[file mode](../../../crates/yo-cli/src/execution/tools/filesystem.rs),
[command process](../../../crates/yo-cli/src/execution/tools/command/process.rs),
[image host](../../../crates/yo-cli/src/execution/image/host.rs).

## 7. 변경 패턴으로 확인하는 경계

| 변경 | Pi의 주요 변경 위치 | Codex의 주요 변경 위치 | Yo에서 허용할 파급 / 피할 파급 |
|---|---|---|---|
| 새 frontend | mode/runtime과 필요한 UI extension adapter | client/protocol과 frontend | client adapter+renderer. Engine/connector에 frontend 분기를 추가하지 않음 |
| 새 wire dialect | pi-ai provider/API + runtime composition | provider/API transport/config의 해당 부분 | connector+명시적 factory/capability. service/protocol에 provider 분기를 추가하지 않음 |
| theme 변경 | interactive theme과 extension/resource surface | TUI rendering | TUI만 변경. host factory 입력이나 실행 snapshot을 바꾸지 않음 |
| 병렬 tool 정책 변경 | agent-loop 실행 함수, Session event 저장과의 연결 | ToolCallRuntime과 turn ordered drain | managed slots와 host claims. Journal writer나 UI별 scheduler를 늘리지 않음 |
| 연결 끊김 | RPC client/process lifetime | app-server client/runtime lifecycle | attachment만 해제. Session 유지 또는 shutdown은 host owner의 명시적 policy에 따름 |

추출 순서는 CLI 내부 neutral preparation→print/TUI의 개별 소비→실제 GUI/web 도입 시 host 이동
→service-owned Session과 local/remote client→versioned DTO/transport로 둔다.
Managed/permissions 개선은 이 host 이동이 끝날 때까지 기다리지 않는다. 각 단계의 구체적인 검증 조건은 다음과 같다.

- CLI start/print/submit/approval/interrupt/resume/shutdown의 기존 동작과 후보 준비 실패 시 기존 Session 보존.
- production import/dependency 검사, local-only consumer build, server-feature consumer build. Theme 변경이 host API에 영향을 주지 않는지 확인.
- queue admission 전 상태와 remote 결과 불명, disconnect와 Session 종료, gap과 terminal 결과를 구분하는 consumer fixture.
- ToolBatch의 역순 완료·publication 전 cancel·첫 capacity 초과·late result에서 writer와 retirement 순서 확인.
- worker lock 범위 축소 시 예약 rollback/stale steer/exact interrupt, 모든 model dispatch 경로의 budget permit과 protected receipt 검증.

이는 구현 시 완료 조건이며 이번에 실행한 테스트가 아니다.
[frontend boundary](../../../methexis/knowledge/agent-runtime/agent.core.frontend-independent-boundary.md)와
[execution topology](../../../methexis/knowledge/agent-runtime/agent.backend.execution-topology.md)는 현재 CLI를
concrete construction owner로 지정한다. Host 추출 시에는 executable의 최상위 wiring/process 정책과
재사용 가능한 construction의 factory 생성/injection으로 소유권 변경 내용을 명시한다.
[기존 Host 계약](../../../methexis/knowledge/agent-runtime/agent.remote.yo-host.md)의 의미는 유지하며,
실제 remote reader가 생기기 전에 범용 reader port를 추가하지 않는다.

## 검토 결과의 취급

독립 read-only 팀은 composition, client, state의 세 관점에서 대응하는 Pi/Codex 원본을 확인했다.
채택한 지적은 neutral config/result, client owner, private state, publication barrier,
host-owned memory CRUD, dispatch 전 budget 경계다. 대량 crate 분할, 거대한 Session/service bag,
모든 core store의 이동은 채택하지 않는다. Journal adapter 정리는 실제 schema 변경 시 검토할 사항으로 남긴다.
수정 후 문서의 재검토와 정적 문서 검사 결과는 작업 보고에서 구분해 제시한다.
