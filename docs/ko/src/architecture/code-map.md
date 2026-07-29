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
        ↕ bounded command and event lanes
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
| [`src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs) | 인자 해석, 터미널 획득 전 표시 방식 선택, 작업 디렉터리 확보, provider 시작, 최상위 정리 결과 취합 | 에이전트 의미나 터미널 렌더링 |
| [`src/agent/mod.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/agent/mod.rs) | `yo-core::AgentSession`을 TUI의 `AgentConnection` 포트에 맞게 연결 | provider 프로토콜 변환 |
| [`src/process/termination`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/termination/mod.rs) | Unix signal 설치·관찰·복원과 마지막 처리 | 터미널 상태 복원 |

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
| [`command.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/command.rs), [`event.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/event.rs), [`session.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/session.rs) | provider에 독립적인 command, 관찰 가능한 event와 outcome, typed identity | 허용되는 상태 전이는 `engine` |
| [`engine`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/engine/mod.rs) | 결정론적인 Session, Turn, Activity, request 상태 전이 | 전이가 provider 경계도 지난다면 `runtime` |
| [`runtime`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/runtime/mod.rs) | 의미 상태를 commit하기 전에 backend 수락을 먼저 처리하는 순서, backend 관찰 결과 변환, 실패 시 활성 작업 종료 | provider port는 `backend/contract.rs` |
| [`agent_session`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/mod.rs) | frontend를 막지 않는 접근, 크기가 제한된 command·event lane, worker 소유권, 시작 취소, 종료 조율 | worker가 소유한 의미 처리는 `runtime` |
| [`backend/contract.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/contract.rs) | provider capability, command, semantic event, polling, 취소, failure kind, 명시적 정리 | 구체적인 adapter |
| [`backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs) | `codex app-server` 생명주기, JSON transport와 protocol 분류, provider ID 연결, core event로 변환 | 새 provider 동작을 노출하기 전 `backend/contract.rs` |

`AgentBackend`가 현재 provider 교체 지점이며 Codex wire value는
`backend/codex` 아래에 있다.
[command와 event 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.command-event-boundary.md)와
[Codex app-server adapter](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md)가
각 동작 제약을 소유한다.

## yo-tui: 터미널 frontend

[`src/lib.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/lib.rs)는
실행 중인 session을 위한 runner facade를 좁게 유지하면서, 완성된 화면
상태와 터미널 연산, HTML Projection에 재사용할 타입을 공개한다.

| 모듈 | 소유하는 책임 | 다음 탐색 지점 |
|---|---|---|
| [`runner`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/mod.rs) | 실행 중인 session의 공개 facade, 터미널을 단독 소유하는 loop, input·event 조율, 마지막 정리 결과 보고 | UI의 의미 상태 전이는 `runner/state.rs`, 실행 중 조율은 `runner/unix.rs` |
| [`input`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/input/mod.rs) | 해석이 끝난 semantic key event, 편집 buffer, 설정 가능한 binding, 종료 gesture, prompt 편집 | 보이는 cursor 배치는 `prompt` |
| [`transcript`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/transcript/mod.rs) | 순서가 있는 사용자·에이전트 item, streaming revision, 대화 기록 layout, scroll 상태 | prompt와 조합하는 일은 `shell` |
| [`prompt`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/prompt/mod.rs) | editor 내용과 cursor가 보이는 상태를 측정하고 그리기 | 편집 의미는 `input/editor` |
| [`shell`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/shell/mod.rs), [`layout`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/layout/mod.rs) | 대화 기록과 prompt 영역 배분, 완성된 frame 하나 조합, cursor 위치 보고 | cell 쓰기는 `surface` |
| [`surface`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/surface/mod.rs), [`text`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/text/mod.rs) | adapter에 독립적인 cell 상태, Unicode grapheme과 너비, 경계가 있는 view, diff span, 터미널에 독립적인 text flow | Projection은 `terminal` 또는 `html` |
| [`terminal`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/mod.rs) | typed terminal operation과 ANSI encoding | 표시 정책은 `terminal/mode`, Unix effect는 `terminal/backend` |
| [`terminal/mode`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/mode/mod.rs), [`terminal/backend`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/backend/mod.rs) | 공유 transactional restoration, Inline·Fullscreen presenter, panic routing, crate-private platform boundary | 프로세스 signal 정책이 바뀔 때만 `yo-cli/process` |
| [`html`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/html/mod.rs) | 완성된 `Surface` 상태를 결정론적으로 브라우저에 Projection | 터미널과 브라우저 출력이 다르면 `surface` |

`surface`는 공통으로 완성된 상태다. 터미널과 HTML Projection은 이를
각자 소비하며, 어느 쪽도 다른 쪽의 layout 의미를 정의하지 않는다.

소유자를 골랐다면 [검증](../validation/)을 변경 경계에서 증거로
이어지는 단일 지도로 사용한다. 실제 터미널 동작이 관련되면
[터미널 환경 매트릭스](../validation/terminal-matrix.md)를 따른다.
[Slice](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#slice-contract)를
닫기 전에는 변경이 실제로 지나간 경계에 대해서만 검사를 넓힌다.
