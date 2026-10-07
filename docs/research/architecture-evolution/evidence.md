# Source evidence and comparison

> Status: non-authoritative research input
>
> 조사 기준일: 2026-10-05 (공식 제품 문서 조회: 2026-10-04)

## 조사 기준과 범위

- Yo: `880467b3186ac7ace0111acd37cb1aa334c9dc4c`의 코드와 현재 worktree.
  기존 `CONTRIBUTING.md` 변경은 보존한다.
- Pi: 사용자 요청에 따라 local checkout을 `v1.0.2`,
  `cd32f7725fdbddbaecdff5b1e68491563394e0ca`로 fast-forward했다.
  [공식 릴리스](https://github.com/earendil-works/pi/releases/tag/v1.0.2).
- Codex: 2026-10-05 GitHub latest release 응답과 annotated tag를 확인하고
  공개 원본을 `/tmp`에 내려받아 직접 대조했다. 기준은
  [rust-v0.160.0](https://github.com/openai/codex/releases/tag/rust-v0.160.0),
  commit `a956835d020762cb2b570053af06f643a11c0ecc`, 공개 시각
  `2026-10-01T20:19:13Z`다. Tag 객체
  `79b1b666f2e8551f8abbbca34957227f67f3f553`와 commit은 다르며 아래 source
  링크는 commit을 사용한다. 다운로드 archive SHA-256은
  `21f8821ecb1248e08dd67cc7e98eb9e860676920b1de69981c78cb9c4e4d909b`다.

Code inspection은 구조와 의존성을 확인한다. 실행 속도, 모델 품질과 실제
터미널 pixel parity의 증거가 아니다. 일반 Pi CLI와 experimental durable/server
stack도 구분한다. 모델의 사전학습 능력은 harness 설계만으로 동일해지지 않는다.

최종 팀이 인용 함수와 인접 호출부를 다시 읽은 범위, 누락 보강과 의도적 보류는
[29-layer source reinspection](./layer-review.md)에 기록한다. 아래 비교 요약을 해당
레이어 전체 source를 빠짐없이 확인했다는 주장으로 해석하지 않는다.
기능 요약과 구별되는 [삼자 모듈 설계 비교](./module-comparison.md)는 조립·client·state·provider의
대응 타입과 import 방향, public/private 경계, 파일별 이동 판단을 기록한다.

## Yo에서 확인한 실행 경로

```text
CLI application -> concrete model/backend composition
  -> AgentSession -> worker-owned AgentRuntime -> AgentEngine
  -> managed model/tool loop -> admitted Connector + local ToolExecutionHost
  OR delegated Codex/Grok adapter
  -> committed Session Journal -> frontend projections
```

| 확인한 사실 | 직접 코드 근거 | 설계 판단 |
|---|---|---|
| public Session facade와 frontend-neutral intent가 있음 | [agent_session.rs](../../../crates/yo-core/src/agent_session.rs), [contract](../../../crates/yo-core/src/agent_session/contract.rs) | SDK가 전혀 없다는 진단은 틀림; 조립/API consumer 문서를 보강 |
| worker command/control/change lane이 bounded | 같은 `agent_session.rs`의 capacity와 worker modules | remote service에서도 backpressure와 urgent controls 보존 |
| queue admission과 actual submission acceptance가 별도 | [admission](../../../crates/yo-core/src/agent_session/admission.rs), [worker lifecycle](../../../crates/yo-core/src/agent_session/worker/lifecycle.rs), [submission](../../../crates/yo-core/src/input/submission.rs) | wire ACK·worker 수락·terminal outcome·durability를 분리 |
| Runtime·Engine·loop가 서로 다른 책임을 가짐 | [runtime](../../../crates/yo-core/src/runtime.rs), [engine state](../../../crates/yo-core/src/engine/state.rs), [managed](../../../crates/backends/managed/src/backend.rs) | 새 frontend용 별도 engine 불필요 |
| native config는 기본 system prompt를 사용 | [native assembly](../../../crates/yo-cli/src/execution/model/native.rs), [config default](../../../crates/backends/managed/src/backend.rs) | CLI의 native 경로에 project instructions 연결 필요 |
| skills와 prompt templates는 이미 있음 | [skill reference](../../../crates/yo-core/src/skill_reference.rs), [prompt templates](../../../crates/yo-tui/src/command/prompt.rs) | 지침 부족을 모든 context customization 부재로 확대하지 않음 |
| 기본 도구 5개, shell만 required approval | [registry](../../../crates/yo-cli/src/execution/tools/registry.rs) | search tools 추가가 작은 승인 감소 개선 |
| 한 TurnState에 active tool와 awaiting approval가 하나씩 | [TurnState](../../../crates/backends/managed/src/backend.rs), [advance queue](../../../crates/backends/managed/src/backend/tools.rs) | batch scheduler의 승인 HOL/serial 실행 구조 보강 |
| Core requests는 map, TUI requests는 queue | [engine state](../../../crates/yo-core/src/engine/state.rs), [TUI state](../../../crates/yo-tui/src/runner/state.rs) | Core/UI 전체가 한 요청만 지원한다는 주장은 틀림 |
| binding이 exact turn/call/tool/args/effect/host 검사 | [tool approval](../../../crates/yo-core/src/tool/approval.rs) | reusable grant와 exact authorization을 분리 |
| ToolExecution은 poll/cancel/shutdown/progress port | [execution](../../../crates/yo-core/src/tool/execution.rs) | bounded parallel execution에 기존 port 활용 가능 |
| registry projection equality가 exact resume를 제약 | [registry](../../../crates/yo-cli/src/execution/tools/registry.rs), [resume](../../../crates/backends/managed/src/backend/adapter/resume.rs) | registry 확장 전에 legacy revision 복원 경로 필요 |
| 현재 command tools는 frozen manifest를 이미 가짐 | [command manifest](../../../crates/yo-cli/src/execution/tools/command/manifest.rs) | extension이 전혀 없다는 진단은 틀림; MCP와 lifecycle gap은 별도 |
| context 정책·자동/manual compaction·exact fork가 있음 | [context](../../../crates/backends/managed/src/backend/context.rs), [compaction](../../../crates/backends/managed/src/backend/compaction.rs), [fork](../../../crates/yo-core/src/agent_session/fork.rs) | 메모리 부족을 압축/세션 분기 부재로 설명하지 않음 |
| local persistence에 durability cutoff/pressure/lease가 있음 | [repository](../../../crates/yo-core/src/session_repository.rs), [local](../../../crates/yo-core/src/session_repository/local.rs), [durable journal](../../../crates/yo-core/src/journal/durable.rs) | 복구 강점을 보존; durable task execution은 아직 별도 gap |
| payload-free causal request trace가 있음 | [trace](../../../crates/yo-core/src/request_trace.rs) | trace 부재가 아니라 timing/approval wait 관측 부족 |
| provider catalog와 dialect connector가 분리 | [binding](../../../crates/yo-core/src/model_service/binding.rs), [native connector selection](../../../crates/yo-cli/src/execution/model/native.rs) | 직접 Anthropic/Gemini wire와 compatible endpoint를 구분 |
| optional runtime fields를 엄격하게 제한 | [profile admission](../../../crates/yo-core/src/model_profile_admission.rs) | typed capability-admitted options만 확대 |
| Kimi closed wire 테스트가 존재 | [request tests](../../../crates/connectors/kimi/src/tests/request.rs) | 모든 provider에 sampling field 주입 금지 |
| live RPC serving 경로가 현재 없음 | [CLI parser](../../../crates/yo-cli/src/command/parser.rs), [application](../../../crates/yo-cli/src/application.rs) | GUI·웹 연결 경계를 구체화 |
| deterministic terminal/HTML frame projection | [Surface](../../../crates/yo-tui/src/surface.rs), [HTML](../../../crates/yo-tui/src/html.rs) | frame parity는 web app/server 존재 증거가 아님 |
| UI palette에 semantic roles와 Default/Light/Mono가 있음 | [palette](../../../crates/yo-tui/src/appearance/palette.rs) | 의미 역할은 재사용하고 DOM layout/color resolution은 별도 |
| Unix CI는 manual dispatch compile/clippy 중심 | [workflow](../../../.github/workflows/unix-compile.yml) | 자동 PR test·실제 설치 여정 보강 |

일부 현재 Developer Docs overview는 Codex adapter를 core 소유로 설명하지만 실제
crate는 `crates/backends/delegated-codex`다. 이 리서치는 현재 코드 경계를 사용한다.
Documentation freshness 수정은 구현 순서의 별도 bounded 작업이다.

## 팀 검토에서 보강한 경계의 직접 근거

아래는 현재 코드·accepted 계약에서 확인한 제약과 **제안의 누락**을 연결한 표다.
새 scheduler/API/Goal/memory가 구현됐거나 현재 제품에 해당 결함이 있다는 뜻이 아니다.
최종 선택과 후속 범위는 [팀 설계 결정](./implementation.md#팀-설계-결정)에 있다.

| 현재 확인한 제약 | Source/authority | 제안에 반영한 경계 |
|---|---|---|
| batch output 최대 1,024, per-tool model/retained output 4/8 MiB, 결과 publication source order | [response bounds](../../../crates/yo-core/src/model_connector/types.rs), [managed bounds](../../../crates/backends/managed/src/backend.rs), [finish_tool](../../../crates/backends/managed/src/backend/tools.rs), [local execution owner](../../../methexis/knowledge/agent-runtime/agent.tool.local-execution-boundary.md) | running count 외 4-slot reorder window와 결과 용량 예약; 역순 readiness는 volatile progress |
| 정확한 approval binding은 Turn/call 단위 | [approval](../../../crates/yo-core/src/tool/approval.rs), [Session lineage](../../../methexis/knowledge/agent-runtime/agent.session.continuation-lineage.md) | C는 call/session/persistent; task scope는 F2 identity 뒤 도입, resume grant 별도 재admit |
| mutation mutex는 LocalToolHost instance별 생성 | [filesystem host](../../../crates/yo-cli/src/execution/tools/filesystem.rs) | lease 보장은 같은 coordinator 안; direct process/editor는 external writer |
| 일반 Submit은 현재 active state로 start/steer 분류 | [admission](../../../crates/yo-core/src/agent_session/admission.rs), [active input owner](../../../methexis/knowledge/agent-runtime/agent.runtime.active-turn-input.md), [TUI Queue help](../../../crates/yo-tui/src/command/help.rs) | wire StartIfIdle/exact Steer 구분, stale draft 보존; 기존 TUI Queue 유지 |
| control의 submission correlation 부재, stale interrupt는 빈 events 반환 가능 | [intent/outcomes](../../../crates/yo-core/src/agent_session/contract.rs), [worker controls](../../../crates/yo-core/src/agent_session/worker/commands.rs) | 작은 neutral outcome bridge와 method별 완료 경계; enqueue≠worker acceptance |
| remote 계약은 WS 첫 transport와 Journal suffix/gap recovery 요구 | [Yo Host owner](../../../methexis/knowledge/agent-runtime/agent.remote.yo-host.md), [repository bounds](../../../methexis/knowledge/agent-runtime/agent.storage.session-repository.md) | public suffix, gap+complete snapshot/없으면 unavailable; bounded host intent ledger는 새 저장 delta |
| publication evidence unavailable 가능, write_file에 before image 없음, mutex가 외부 writer를 통제하지 못함 | [file evidence](../../../crates/yo-core/src/event/tool.rs), [mutation](../../../crates/yo-cli/src/execution/tools/filesystem/mutation.rs) | 첫 Changes inspect-only, Revert 후속; 임의 writer에 대한 atomic CAS 주장 없음 |
| fork는 idle/pending 없음/durable parent와 same workspace 필요 | [fork admission](../../../crates/yo-core/src/agent_session/fork.rs), [fork descriptor](../../../crates/yo-cli/src/application/runtime/startup/prepare.rs), [lineage owner](../../../methexis/knowledge/agent-runtime/agent.session.continuation-lineage.md) | active parent Task는 fresh Session+실제 bytes의 bounded seed; 기존 intentional fork 조건 유지 |
| managed만 자기 model dispatch 통제, delegated usage/retry는 사후 notice | [managed dispatch](../../../crates/backends/managed/src/backend/request.rs), [delegated usage](../../../crates/backends/delegated-codex/src/runtime/events/notifications/usage.rs), [retry notice](../../../crates/backends/delegated-codex/src/runtime/events/notifications/turn.rs), [backend capabilities](../../../crates/backends/foundation/src/contract.rs) | hard logical request/round는 managed capability; delegated 내부 rounds/retries/spend는 planning/unknown |
| Session worker sole Journal writer, 현재 SemanticRecord에 Goal/Task/Budget 없음 | [Journal owner](../../../methexis/knowledge/agent-runtime/agent.observability.session-journal.md), [records](../../../crates/yo-core/src/journal/record.rs) | parent agentic records/child TaskOrigin/seed의 closed delta, exact crash join; host map은 derived |
| canonical workspace identity와 counted/retained request 검증 패턴 있음 | [workspace](../../../crates/yo-core/src/host/workspace.rs), [descriptor](../../../crates/yo-core/src/session/descriptor.rs), [accounting fixtures](../../../crates/backends/managed/src/backend/tests/connector_rounds.rs) | worktree memory scope 분리; provisional recall/off admission lock과 counted=sent=recorded=resume |
| schema는 closed subset, ToolExecutionResult는 textual | [schema](../../../crates/yo-core/src/tool/schema.rs), [execution result](../../../crates/yo-core/src/tool/execution.rs) | 첫 MCP는 exact subset+bounded text; unsupported schema/content를 조용히 약화하지 않음 |
| model selection/setup은 Session 생성 전, native coordinates와 delegated runtime selection이 다름 | [model selection owner](../../../methexis/knowledge/agent-runtime/agent.model.session-selection.md), [picker coordinates](../../../crates/yo-core/src/model_service/selection/coordinates.rs), [runtime flow](../../../docs/src/architecture/runtime-flow.md) | native admitted target/delegated opaque verified account+catalog evidence, TTY setup+Refresh; implicit model·client-local remote path 없음 |

## Pi의 적용 가능한 구조와 한계

Pi source root는 local `/home/yon/projects/pi`이며 아래 GitHub source 링크는
조사한 exact commit에 결합된다.

| 관측 | Source | Yo 적용 판단 |
|---|---|---|
| 일반 CLI는 SDK→AgentSession→Agent 경로 | [sdk](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/sdk.ts), [AgentSession](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/agent-session.ts) | 쉬운 assembly API 참고; 큰 Session 객체 전체를 복제하지 않음 |
| 지침·skills·tool guidelines를 system prompt sections로 조합 | [resource loader](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/resource-loader.ts), [system prompt](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/system-prompt.ts) | provenance-bearing instruction snapshot의 좋은 선행 사례 |
| Agent 기본 tool execution은 parallel | [agent](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/agent/src/agent.ts), [loop](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/agent/src/agent-loop.ts) | parallel 기본값만 복사하면 권한/resource proof가 빠짐 |
| batch 준비는 순차 await, 실행은 Promise.all | 같은 loop의 `executeToolCallsParallel` | beforeToolCall approval hook은 batch 시작을 막을 수 있음; 승인 HOL 해결 증거가 아님 |
| 같은 파일 mutation queue가 있음 | [file queue](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/tools/file-mutation-queue.ts) | host canonical lease를 참고; shell 효과까지 자동 독립이라고 보지 않음 |
| grep/find/ls 도구가 있으나 default enabled는 read/bash/edit/write | [tools](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/tools/index.ts), [settings](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/settings-manager.ts) | typed search는 구현 사례이며 default parity로 과장하지 않음 |
| MCP/codemode는 일반 CLI에 연결됨 | [MCP extension](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/extensions/mcp/tools.ts), [MCP client](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/mcp/src/index.ts), [codemode](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/codemode/README.md) | tool-only integration 먼저; full JS orchestration은 비용 측정 뒤 |
| Models/Provider에 capability/options/auth 구성 | [model runtime](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/model-runtime.ts), [provider composer](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/provider-composer.ts) | 오래된 api-registry 구조를 최신이라고 사용하지 않음 |
| extension이 tool/command/provider/UI hook을 등록 | [extension types](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/extensions/types.ts) | lifecycle를 재사용하되 UI hook을 core 실행 authority로 만들지 않음 |
| RPC는 기존 Session을 stdio JSON 메시지로 노출 | [RPC mode](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/modes/rpc/rpc-mode.ts) | 새 실행 엔진 없이 좁은 API를 시작하는 근거 |
| 일반 SessionManager는 append/tree rewrite | [SessionManager](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/core/session-manager.ts) | experimental durable의 strong transaction 성질과 구분 |
| durable/server/client/chord는 experimental 분산/복구 경로 | [durable](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/durable/README.md), [server](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/server/README.md) | 일반 CLI와 연결됐다는 근거 없이 전체 분산 stack을 따라 만들지 않음 |
| cross-model message transform은 일부 내용·signature를 변환/삭제 | [transform](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/ai/src/api/transform-messages.ts) | 손실 있는 portability를 Yo exact replay보다 강한 보장으로 설명하지 않음 |
| telemetry는 명시 context의 별도 package | [telemetry](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/telemetry/README.md) | bounded passive timing port 참고 |
| 기본 도구는 OS 권한을 상속; trust는 지침/hook 로딩 문제 | [security](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/docs/security.md) | unrestricted 실행을 승인 fatigue 해결로 채택하지 않음 |

## Codex 원본에서 확인한 실행 구조

공개 Rust workspace의 실행·권한·병렬 처리·지침·저장·메모리·goal/child·MCP·
protocol/transport·TUI·provider·CI 경계를 직접 읽었다. 아래는 그 경계를
대표하는 코드와 적용 판단이다. 전체 source의 모든 줄을 검토하거나 upstream을
빌드·실행한 감사가 아니다. 공개 Rust TUI와 App Server의 증거를 비공개 desktop/
browser 제품 전체의 구현 증거로 확장하지 않는다.

| 직접 확인한 구조 | 고정된 원본 source | Yo 적용 판단 |
|---|---|---|
| CodexThread가 submission/steer/suspend/events를 노출하고 core Session/turn이 model/tool loop를 소유 | [thread](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/codex_thread.rs), [loop](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/session/turn.rs) | Yo AgentSession/worker/managed 보존; frontend별 engine 불필요 |
| 공통 tools, extension contributors, protocol/transport, provider가 core 외부 경계를 가짐 | [tools](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tools/src/lib.rs), [extension API](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/ext/extension-api/src/lib.rs) | 소유 분리는 참고하되 같은 crate 수와 범용 bus를 복제하지 않음 |
| ToolCallRuntime은 parallel-capable call에 read lock, 나머지에 write lock을 잡고 dispatch 종료 때 해제 | [parallel](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/tools/parallel.rs), [router](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/tools/router.rs) | parallel flag와 effect 독립성은 다름; host resource claims로 충돌·순서 명시 |
| dispatch 안 orchestrator가 approval을 await하므로 exclusive call의 approval wait가 gate를 점유할 수 있음; exec_command 자체는 parallel-capable | [orchestrator](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/tools/orchestrator.rs), [exec](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/tools/handlers/unified_exec/exec_command.rs) | 모든 승인 HOL을 없앤 선례는 아님; approval/ready/running과 pending mutable claim 순서를 함께 설계 |
| policy의 approval skip과 strict auto-review가 별도이며 strict mode는 skip action도 review하고 sandbox 밖 retry를 fresh review | 같은 [orchestrator](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/tools/orchestrator.rs) | 추가 model reviewer를 매 call 기본값으로 삽입하지 않음; 증명된 host policy 우선 |
| 플랫폼 sandbox 선택·permission transform·managed network가 실행 경계에 있음 | [sandbox manager](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/sandboxing/src/manager.rs), [permission DTO](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/app-server-protocol/src/protocol/v2/permissions.rs) | actual OS enforcement와 capability 검증; approval만으로 confinement 주장 금지 |
| root→cwd instruction chain, override, byte budget, trust/provenance snapshot, serialized refresh가 있음; discovery는 symlink 허용 | [discovery](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/agents_md.rs), [manager](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/agents_md_manager.rs) | provenance 참고; Yo no-follow와 exact historical resume는 별도 보존 |
| local/remote/model-free token-budget compaction 경로와 replacement history/checkpoint metadata가 구분됨 | [compact](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/compact.rs), [token-budget compact](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/compact_token_budget.rs) | fresh window를 portable summary/exact continuation과 같은 보장으로 취급하지 않음 |
| JSONL writer에 command/ack, persist/flush/shutdown, pending write recovery가 있음 | [recorder](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/rollout/src/recorder.rs) | JSONL이라는 이유로 recovery 부재라 진단하지 않음; Yo durable cutoff/lease 보존 |
| active API는 v2 DTO와 TS/JSON schema export; stdio/Unix socket/WS 별도 transport | [protocol](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/app-server-protocol/src/lib.rs), [transport](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/app-server-transport/src/transport/mod.rs) | Yo v1은 Yo 의미의 DTO; Codex v2를 그대로 public 계약으로 사용하지 않음 |
| listener lane에서 live resume/subscription을 순서화; ordinary pending server requests replay와 one-shot callback 제거 | [thread state](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/app-server/src/thread_state.rs), [listener](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/app-server/src/request_processors/thread_lifecycle.rs), [callbacks](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/app-server/src/outgoing_message.rs) | gapless attach 참고; Yo controller generation은 추가 설계이며 upstream 보장으로 주장하지 않음 |
| revocable connection gate가 queued handler를 막되 시작한 handler cleanup 허용; slow disconnectable client는 outbound full 때 종료 | [RPC gate](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/app-server/src/connection_rpc_gate.rs), [routing](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/app-server/src/transport.rs) | admission과 completion 분리; queue count와 payload byte budget 모두 필요 |
| raw WS listener는 Origin header를 거절하고 upgrade auth 검사; capability/signed bearer와 non-loopback 무인증 경계 | [WS](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/app-server-transport/src/transport/websocket.rs), [auth](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/websocket-auth/src/lib.rs) | native-client WS가 곧 browser transport는 아님; Yo browser에는 별도 cookie/Origin 정책 |
| TUI가 embedded in-process App Server 또는 local daemon/remote target을 사용 | [TUI assembly](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/lib.rs), [connection](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/app_server_connection.rs) | service 의미 공유 가능; daemon/network serialization 강제 불필요 |
| daemon은 persistent root IDs를 저장하고 shared cold resume로 재적재 | [recovery](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/app-server/src/daemon_thread_recovery.rs) | saved Session 복구와 진행 중 external effect 재실행 구분 |
| child는 capacity reservation·권한 intersection·initial input 수락 후 commit; pending spawn 취소는 child와 edge 정리 | [spawn](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/agent/control/spawn.rs), [guard](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/agent/control/spawn_guard.rs) | partial creation failure도 task state와 bounded cleanup으로 설계 |
| root tree rollout budget에 reported usage 누적; Goal extension은 serialized state/accounting과 descendant usage 사용 | [rollout budget](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/rollout_budget.rs), [Goal runtime](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/ext/goal/src/runtime.rs), [accounting](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/ext/goal/src/accounting.rs) | reported-usage 누적은 dispatch 전 hard reservation 증거가 아님; hard/advisory 구분 유지 |
| memory writer가 root/feature/state DB 조건에서 phase1→phase2 실행; leased parallel model extraction·redaction, global consolidation lease·filesystem sync·agent | [start](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/memories/write/src/start.rs), [phase1](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/memories/write/src/phase1.rs), [phase2](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/memories/write/src/phase2.rs) | claims/read-write 분리 참고; 첫 memory에 DB queue·추가 model pipeline 요구하지 않음 |
| memory read/citation/usage와 write pipeline이 별도 crate; extension이 use config/bounded fragments와 dedicated tools 제공 | [read](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/memories/read/src/lib.rs), [extension](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/ext/memories/src/extension.rs), [write owner](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/memories/write/src/lib.rs) | recall/contribution은 모듈 분리; memory text를 permission authority로 승격하지 않음 |
| MCP catalog revision/cached-live readiness 구분; handler는 server opt-in 또는 readOnlyHint로 parallel 판정 | [catalog](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/codex-mcp/src/connection_manager/tool_catalog.rs), [MCP handler](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/tools/handlers/mcp.rs) | schema snapshot 참고; hint만으로 independence/권한을 증명하지 않는 Yo host admission |
| codemode nested dispatch도 ToolCallRuntime 사용; provider capability가 optional features의 upper bound | [codemode](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/tools/code_mode/mod.rs), [provider](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/model-provider/src/provider.rs) | 내부 call도 같은 admission; sampling/remote compaction을 모든 dialect에 강제하지 않음 |
| PR blocking entry와 changed-path Rust checks가 연결됨 | [PR entry](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/.github/workflows/blocking-ci.yml), [Rust checks](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/.github/workflows/rust-ci.yml) | affected PR test/install smoke 참고; upstream 전체 CI 복제 불필요 |

메모리 directory README는 orchestration owner를 `core/src/memories`로 설명하지만
이 tag에서 그 경로는 없다. 실제 owner는 위 `memories/write/src/{start,phase1,
phase2,runtime}.rs`다. Overview보다 현재 코드로 판단했다. Phase2의 managed
local/no-network profile도 parent의 명시적 Disabled/External profile을 무조건
managed로 바꾸지는 않는다. 모든 memory worker의 동일 OS confinement 보장으로
일반화하지 않는다.

## Codex에서 확인한 제품·protocol 경계

현재 일반 Codex 지원 minor와 image/secret 전용 검증 버전은 서로 다르다.
Yo의 [initialize parser](../../../crates/backends/delegated-codex/src/protocol/initialize.rs)는
일반 verified minor `0.145/0.146/0.149`, image `0.153.4/0.154.0`을 별도로 취급한다.
[secret probe](../../../crates/backends/delegated-codex/src/runtime/secret_probe.rs)는
`0.155.1` exact wire를 요구한다. 이들은 현재 upstream latest version의 증거가 아니다.

| 확인한 사실 | 증거 | Yo 판단 |
|---|---|---|
| App Server가 기존 agent를 bidirectional protocol로 노출 | [OpenAI App Server](https://learn.chatgpt.com/docs/app-server), Yo [client](../../../crates/backends/delegated-codex/src/client.rs) | Session API를 frontend별 engine 없이 제공 |
| stdio JSONL, version별 schema 생성, WS/remote 경계 | 같은 공식 문서 | explicit initialization와 generated DTO/schema; WS는 문서상 experimental |
| approval response는 offered choice와 outstanding request에 결합 | Yo [response](../../../crates/backends/delegated-codex/src/runtime/input/response.rs), [requests](../../../crates/backends/delegated-codex/src/runtime/events/requests.rs) | 임의 session grant를 delegated host에 발명해 보내지 않음 |
| OS sandbox로 routine autonomous work 경계를 제한 | [OpenAI Sandbox](https://learn.chatgpt.com/docs/sandboxing) | scope reuse에는 실제 effect enforcement가 필요 |
| local memory와 required team instruction은 다른 layer | [OpenAI Memories](https://learn.chatgpt.com/docs/customization/memories) | memory를 지침/권한 authority로 승격하지 않음 |
| subagents는 별도 threads와 결과 수집 | [OpenAI Subagents](https://learn.chatgpt.com/docs/agent-configuration/subagents) | bounded child sessions와 result/artifact reference |
| worktree는 독립 chat의 파일 변경을 분리 | [OpenAI Worktrees](https://learn.chatgpt.com/docs/environments/git-worktrees) | task workspace isolation, 별도 sandbox와 shared Git locks |
| goal은 outcome/constraints/completion criteria를 지속 | [OpenAI Long-running work](https://learn.chatgpt.com/docs/long-running-work) | typed goal과 evidence-based completion |
| 원격 화면은 연결된 컴퓨터의 일을 지시·승인·검토 | [OpenAI Remote](https://learn.chatgpt.com/docs/remote) | mobile control surface, host-side execution 유지 |

공식 제품 기능은 공개된 사용 경험의 증거다. Codex desktop/mobile/cloud의 비공개
소스 전체나 multi-tenant 운영 구현을 확인한 증거가 아니다. Yo의 제안 authentication,
grant와 memory 저장 모양은 이러한 경계에서 얻은 설계 판단이다.

## 전 레이어 적용 지도

| 레이어 | Yo 현재 기반 | 우선 gap와 목표 | 재사용/차별점 |
|---|---|---|---|
| product entry/install | Unix CLI/print | clean install/upgrade path와 public SDK assembly | 초기 daemon 강제 금지 |
| composition | CLI-owned concrete assembly | independent clients 때 host 추출 | concrete deps가 core로 역류하지 않음 |
| identity/workspace | Host UUID, canonical workspace | service attachment/controller IDs | display path로 실행 host를 추측하지 않음 |
| semantic engine | Session/Turn/Activity/Request | 새 goal/task 의미의 작은 확장 | 상태 전환 한 owner |
| managed loop | strict connector/model/tool sequencing | ToolBatch scheduler | stable source-order replay |
| providers/connectors | dialect split, typed profiles | capability/options/auth와 직접 dialect coverage | no probe/fallback, private replay 격리 |
| instruction/context | explicit skills/references + compact | project instruction snapshots | exact resume의 과거 계약 유지 |
| accounting/cache | typed request accounting, usage/cache receipt | timing/cost availability UX | exact와 image advisory quality 유지; cache hit는 occupancy 감소가 아님 |
| basic tools | bounded batch read/atomic write/shell | typed path/content search, artifacts/diff | host no-follow/admission 유지 |
| tool authorization | exact call approval | scoped grants + fresh exact authorization | workflow approval와 runtime policy 분리 |
| execution isolation | filesystem confinement, approved process | capability-tested process sandbox | approval 자체를 sandbox라 부르지 않음 |
| concurrency | one active native tool | independent reads; later proven writes | opaque shell/MCP exclusive |
| extension/MCP | frozen external command tools | native MCP/tool discovery | server identity/schema epochs |
| codemode | 없음 | tool catalog 비용이 클 때 선택 도입 | 모든 내부 call 권한·budget 검사 |
| steering/questions | active steer, requests/interview | client-independent controller routing | exact request·generation |
| plan/goal | summary의 objective, Turn loop bounds | typed goal/plan completion criteria | model text는 완료 증거 아님 |
| multi-agent/task | native child task coordinator 없음 | bounded independent child sessions/worktrees | dependency/result scope와 ownership |
| compaction | auto/manual portable checkpoint | structured goal/memory context 연결 | source groups와 admitted accounting policy/quality |
| long-term memory | explicit local skills, Session history | scoped entries/recall/forget | files authority, index rebuildable |
| persistence/recovery | anchored exact replay/fork/pressure | operation/task crash classification | uncertain effects 자동 재실행 금지 |
| portability | explicit exact model boundaries | disclosed lossy migration | Pi transforms의 loss를 구분 |
| TUI | deterministic cells, rich text/media, SSH | task/changes/request progress 정보 구조 | 검증된 renderer 보존 |
| GUI | core 재사용 경계만 있음 | shared DOM UI + thin desktop wrapper | 두 번째 engine 없음 |
| browser/remote | frame HTML projection만 있음 | authenticated host API, SSH→TLS | HTML snapshot은 web product 아님 |
| multimodal/artifact | bounded image input, Kitty/cell output | frontend upload/artifact IDs and diffs | arbitrary path/URL fetch 금지 |
| diagnostics/telemetry | typed payload-free request trace | bounded timing/causal spans/recovery actions | passive observer |
| testing/evals | extensive fixtures, selected integration | representative task/approval/recovery evals | static audit를 speed proof로 사용하지 않음 |
| CI/release | manual Unix compile/clippy workflow | automatic PR selected suites/install smoke | product platform 범위 유지 |
| docs/workflow | owner routing, Methexis, root budgets | stale overview 개선, ordinary loop 간결화 | semantic authority를 research로 대체하지 않음 |

## 재검증 조건

Yo HEAD/registry/protocol 지원 버전, Pi tag 또는 Codex source pin이 바뀌면 해당
source 행을 다시 확인한다. Original-source와 official-product evidence는 계속
구분한다. 링크 존재 여부는 source 내용·실제 runtime 보장·최신 버전 호환성의
검증이 아니다. Codex `0.160.0` source audit만으로 Yo delegated adapter의 verified
minor/image/secret 범위를 넓히지 않는다. 새 버전 지원은 wire fixtures와 실제
초기화·approval·stream·resume·image/secret conformance를 별도로 확인해야 한다.
