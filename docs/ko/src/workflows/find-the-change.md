# 변경 지점 찾기

익숙한 파일 이름이 아니라 관찰 가능한 결과에서 시작한다. 이 페이지는
처음 확인할 소유자와 그다음 경계를 고르는 데 사용한다. 모듈 책임은
[코드 지도](../architecture/code-map.md), 여러 크레이트의 실행 순서는
[실행 흐름](../architecture/runtime-flow.md), 검사 결과의 의미는
[검증](../validation/)을 참고한다.

동작 계약은 Methexis가 소유한다. 아래 경로는 계약을 다시 정의하지 않고
해당 소유자로 연결한다.

## 검색 깊이 선택하기

경계를 골랐다면 질문에 답할 수 있는 가장 저렴한 검색부터 사용한다.

| 질문 | 사용할 도구 | 이유 |
|---|---|---|
| 이 타입, 함수, 오류 문구, event variant의 정확한 이름은 어디에 있는가? | `rg` | 색인이 필요 없는 빠른 문자열 또는 정규식 검색 |
| 이 symbol을 어디서 정의하고 어떤 reference가 실제로 연결되는가? alias와 macro 확장 뒤에도 무엇이 남는가? | [rust-analyzer의 definition·reference 탐색](https://rust-analyzer.github.io/book/features.html) | 단순 문자열 일치가 아니라 Rust 프로젝트 의미를 사용 |
| 이름이나 formatting이 달라도 같은 syntax 형태가 어디에 있는가? | [ast-grep 읽기 전용 구조 검색](https://ast-grep.github.io/reference/cli/run.html) | parsing된 Rust node를 코드와 비슷한 pattern 및 metavariable로 검색 |
| 검색 결과는 저장소의 어느 책임에 속해야 하는가? | 이 페이지와 [코드 지도](../architecture/code-map.md) | AST 형태와 symbol resolution만으로 아키텍처 소유권을 결정할 수 없음 |

`rust-analyzer`는 이미 `rust-toolchain.toml`에 선택되어 있다. 의미 기반
탐색에는 editor/LSP의 definition과 reference 기능을 사용한다.
rust-analyzer가 불안정하다고 명시한 CLI subcommand는 저장소의 공식
interface로 삼지 않는다.

ast-grep은 필수 저장소 도구가 아니라 선택적인 탐색 보조 도구다. 구체적인
타입 이름과 상관없이 trait 구현을 찾는 읽기 전용 검색은 다음과 같다.

```bash
ast-grep run --lang rust \
  --pattern 'impl $TRAIT for $TYPE { $$$BODY }' crates
```

syntax 중심의 outline도 만들 수 있다.

```bash
ast-grep outline --lang rust crates/yo-core/src
```

구조 검색은 parsing된 syntax를 이해하지만 Rust type resolution이나 macro
의미까지 이해하지는 않는다. 중요한 결과는 rust-analyzer와 소유 모듈의
test로 다시 확인한다. 저장소 탐색에서는 구조 rewrite를 사용하지 않는다.
실행 파일은 `ast-grep`이라는 정확한 이름을 쓴다. `sg` alias는 전혀 다른
시스템 명령일 수 있다.

### 탐색 pilot 결과

ast-grep 0.45.0으로 읽기 전용 pilot을 실행해 현재 workspace에서 세 가지
검색을 비교했다.

| 질문 | ast-grep 결과 | `rg` 비교 | 판단 |
|---|---:|---:|---|
| 어떤 타입이 `AgentBackend`를 구현하는가? | 6 | 6 | 정확한 text 형태가 일정해 `rg`가 더 간단하다. |
| `if let Err(...)`로 failure를 처리하는 곳은 어디인가? | 17 | 17 | 구조 검색이 의미 있는 구분을 더하지 못했다. |
| 인자 없는 `shutdown()` method를 호출하는 곳은 어디인가? | parsing된 호출 67개 | text 행 73개 | assertion macro 내부 호출 6개를 text 검색에서 추가로 찾았다. 구조 pattern은 이 macro token body를 검사하지 못했다. |

`ast-grep outline`은 `yo-core` source file 26개의 간결한 목록을 만드는 데
유용했지만, 저장소 소유권을 정하거나 Rust symbol을 resolve할 수는 없다.
따라서 이 pilot에서는 package, configuration, 고정 version, 저장된 query를
추가하지 않았다.

첫 검색은 계속 `rg`를 사용한다. 익숙하지 않은 모듈을 빠르게 훑을 때는
선택적으로 ast-grep outline을 사용하고, text 검색으로 syntax 형태를
깔끔하게 표현할 수 없을 때만 일회성 읽기 전용 구조 query를 사용한다.
macro 내부 발생은 `rg`로 확인하고 symbol 의미는 rust-analyzer로 확인한다.
실제 Slice에서 같은 구조 query가 반복해서 필요하고, query가 잡음을
실질적으로 줄이며, macro 사각지대를 확인할 보조 검사가 있을 때만 저장소
도구의 version 고정을 다시 검토한다.

## 첫 소유자 선택하기

| 원하는 결과 | 시작 지점 | 다음 경계로 이동하는 조건 | 계약 소유자 |
|---|---|---|---|
| 해석된 key, 편집, paste, 설정 가능한 binding, 종료 gesture 변경 | [`yo-tui/src/input`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/input/mod.rs) | 보이는 cursor 측정도 바뀌면 `prompt`, semantic agent action도 바뀌면 `runner` | [활성 Turn input](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.active-turn-input.md) |
| prompt 줄 바꿈, cursor 표시, prompt viewport 변경 | [`yo-tui/src/prompt`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/prompt/mod.rs) | 영역 배분도 바뀌면 `shell` 또는 `layout`, 편집 의미도 바뀌면 `input` | [경계가 있는 view](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.bounded-view.md) |
| 대화 기록 item, streaming update, scroll, page 이동 변경 | [`yo-tui/src/transcript`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/transcript/mod.rs) | 대화 기록과 prompt 배분도 바뀌면 `shell`, event 해석도 바뀌면 `runner/state.rs` | [typed TUI 흐름](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.typed-flow.md) |
| Chat/Transcript/Request 선택, 정확한 Request anchor, 읽기 전용 강제, compact mode chrome, view별 context·scroll 복원 변경 | [`yo-tui/src/runner/view.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/view.rs) | key mapping 정책도 바뀌면 `input/view_binding.rs`, semantic record shape나 correlation도 바뀌면 `yo-core` | [observation view Projection](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.observability.view-projections.md) |
| shell 영역이나 완성된 frame 조합 변경 | [`yo-tui/src/shell`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/shell/mod.rs)과 [`layout`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/layout/mod.rs) | cell 쓰기나 clipping도 바뀌면 `surface`, 터미널 effect도 바뀌면 `terminal` | [Surface geometry](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.geometry.md) |
| grapheme 너비, cell 소유권, clipping, 결정된 style, diff span 변경 | [`yo-tui/src/surface`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/surface/mod.rs)와 [`text`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/text/mod.rs) | Projection끼리 다르면 `terminal` 또는 `html`, 조합 정책도 바뀌면 호출한 component | [Surface model](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.model-ownership.md) |
| 완성된 frame에서 생성되는 HTML 변경 | [`yo-tui/src/html`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/html/mod.rs) | 공통 상태에 대해 터미널과 HTML이 다르면 `surface`, HTML encoding만 다르면 이 모듈에 유지 | [HTML Projection](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.html-projection.md) |
| ANSI encoding이나 typed terminal operation 변경 | [`yo-tui/src/terminal`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/mod.rs) | 화면 정책도 바뀌면 `terminal/mode`, OS effect도 바뀌면 `terminal/backend` | [터미널 operation](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.terminal-ops.md) |
| Inline·Fullscreen 표시, viewport update, 복원 변경 | [`yo-tui/src/terminal/mode`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/mode/mod.rs) | Unix 획득이나 출력도 바뀌면 `terminal/backend`, 프로세스 signal 동작도 바뀌면 `yo-cli/process` | [생명주기 복원](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.lifecycle-restoration.md) |
| live loop의 input·event 순서, backpressure 처리, TUI event Projection 변경 | [`yo-tui/src/runner`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/mod.rs) | Session admission이나 event 의미도 바뀌면 `yo-core/agent_session`, 터미널 정책도 바뀌면 `terminal/mode` | [typed TUI 흐름](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.typed-flow.md) |
| Session, Turn, Activity, request, command, event의 의미 변경 | [`yo-core/src/engine`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/engine/mod.rs), [`command.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/command.rs), [`event.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/event.rs) | provider 수락이나 관찰이 관련되면 `runtime`, frontend concurrency가 관련되면 `agent_session` | [Session 생명주기](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.session-turn-activity.md) |
| frontend admission, backpressure, worker 소유권, 시작 취소, 종료 변경 | [`yo-core/src/agent_session`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/mod.rs) | semantic transition도 바뀌면 `engine` 또는 `runtime`, TUI gesture도 바뀌면 `yo-tui/runner`로 돌아감 | [command와 event 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.command-event-boundary.md) |
| provider 중립 backend port나 command 수락 순서 변경 | [`yo-core/src/backend/contract.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/contract.rs)와 [`runtime`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/runtime/mod.rs) | Codex wire 동작도 바뀌면 `backends/delegated-codex`, 공개 frontend 사용법도 바뀌면 `lib.rs`와 `agent_session` | [프런트엔드 독립 코어](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.core.frontend-independent-boundary.md) |
| Codex process, JSON protocol, version gate, ID 연결, event 변환 변경 | [`backends/delegated-codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-codex/src/lib.rs) | provider 중립 의미도 바뀌면 `backend/contract`, `runtime`, `engine` | [Codex adapter](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md) |
| CLI 인자, 작업 디렉터리 확보, 시작 순서, 최상위 failure 취합 변경 | [`yo-cli/src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs) | agent adapter도 바뀌면 `agent`, signal 정책도 바뀌면 `process/termination` | [모듈 및 호스트 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.architecture.module-boundaries.md) |
| `Ctrl+Z`, 기본 프로세스 일시정지, `SIGCONT`, 터미널 세대 재진입 변경 | [`yo-cli/src/process/job_control.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/job_control.rs)와 [`yo-tui/src/runner`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/mod.rs) | 보존하는 application 상태도 바뀌면 `runner/session.rs`, lease 최종 확정에서 종료가 우선하면 `process/termination` | [job-control 일시정지와 재개](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.job-control-suspend-resume.md) |
| Unix 종료 관찰, signal 우선순위·disposition·복원 변경 | [`yo-cli/src/process/termination`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/termination/mod.rs) | 터미널 복원도 바뀌면 `yo-tui/terminal/mode`, typed 관찰도 바뀌면 `yo-tui/runner` | [프로세스 종료 coordinator](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.process-termination-coordinator.md) |

설치된 Codex minor line이 거부되거나 schema가 바뀌었다면 version gate를
변경하기 전에
[Codex app-server upstream 따라가기](./codex-upstream.md)를 사용한다.

소유자를 고른 뒤
[변경 경계별 검사](../validation/#변경된-경계에서-시작하기)에서
첫 검사를 선택한다. 변경을 승인하기 전
[Slice 종료 기준선](../validation/#slice-종료-기준선)과 영향을 받은
환경 매트릭스를 실행한다.

## 여러 경계를 따라 증상 추적하기

보이는 실패를 표시한 모듈이 실제 소유자라는 보장은 없다.

| 증상 | 이 순서로 추적 |
|---|---|
| 제출한 text는 보이지만 Turn이 시작되지 않음 | `yo-tui/runner/state.rs` → `yo-cli/agent` → `yo-core/agent_session/admission.rs` → worker/runtime → backend |
| Codex가 작업을 수락했지만 대화 기록이 갱신되지 않음 | `backend/codex/events.rs` → `AgentRuntime::poll_event`와 Journal 추가 → agent-session 변경 알림 → `yo-cli/agent` Transcript cursor → `TuiState::observe_record` → Chat transcript |
| Chat은 갱신되지만 Transcript나 Request가 잘못됨 | `TuiState::observe_record` → `runner/view.rs`의 record formatting과 정확한 context anchor → 공통 `transcript` layout. 필요한 semantic record나 correlation 자체가 없을 때만 `yo-core` 확인 |
| backend가 바쁠 때만 input이 멈춤 | runner pending dispatch → `AgentSession::dispatch`/`retry` → bounded command lane → worker lifecycle |
| 일반 종료 뒤 터미널 상태가 손상됨 | terminal mode guard → presenter cleanup → Unix backend. signal 경로가 관련될 때만 process termination 확인 |
| 정리가 보이기 전에 signal로 종료됨 | TUI typed termination observation → guarded terminal return → agent shutdown → `TerminationCoordinator::with_active_resource` |
| `Ctrl+Z` 뒤 터미널이 손상되거나 `fg` 뒤 Session이 사라짐 | TUI `SuspendRequested` → guarded terminal cleanup → active-resource 최종 확정 → `process/job_control` → 새 터미널 세대 |
| 터미널과 HTML이 다름 | 공유 fixture와 완성된 `Surface` → terminal Projection과 HTML Projection을 각각 확인 |
| tmux, SSH, SSH 내부 tmux에서만 실패함 | 먼저 실제 환경 경로를 재현한 뒤 terminal mode/backend 확인 |

시작, Turn 하나, 정리의 전체 순서는 [실행 흐름](../architecture/runtime-flow.md)을
본다. passed, failed, unverified의 의미는 [검증](../validation/)을
본다.

## 변경을 소유자 안에 유지하기

수정 범위를 넓히기 전에 다음을 확인한다.

1. 원하는 결과가 동작을 바꾸는지 구현 세부사항만 바꾸는지 확인한다.
2. 동작이 바뀌면 연결된 Methexis KnowledgeUnit을 읽는다.
3. Codex JSON은 `backends/delegated-codex`, 터미널 type은 `yo-tui` 안에 둔다.
4. 필요한 동작의 소유자가 따로 있다면 계층을 건너뛰는 지름길을 추가하지
   않고 변경을 해당 소유자에게 옮긴다.
5. 소유자 곁에 실패를 구분할 수 있는 가장 작은 test를 추가한 뒤, 변경이
   실제로 지난 모든 경계까지 검증을 넓힌다.

원하는 결과를 한 행에 맞추려면 둘 이상의 public boundary를 바꿔야 한다면
수정 전에 [코드 지도](../architecture/code-map.md)를 확인한다. 이는 보통
설계 결정이나 Slice 분할이 필요한 신호다. 한 모듈이 다른 모듈의 책임까지
흡수해야 한다는 뜻이 아니다.
