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
| [`src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs) | 인자 해석, 터미널 획득 전 표시 방식 선택, 작업 디렉터리 확보, provider 시작, 터미널 세대 재진입, 최상위 정리 결과 취합 | 에이전트 의미나 터미널 렌더링 |
| [`src/agent/mod.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/agent/mod.rs) | 구체적인 local Transcript cursor를 포함해 `yo-core::AgentSession`을 TUI의 `AgentConnection` 포트에 맞게 연결 | provider 프로토콜 변환 또는 시기상조인 local·remote reader trait |
| [`src/process/job_control.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/job_control.rs) | 기본 `SIGTSTP` 동작 적용, 프로세스 일시정지, `SIGCONT` 뒤 물려받은 signal 상태 복원을 하나의 transaction으로 처리 | TUI 상태나 터미널 복원 |
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
| [`journal`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/journal/mod.rs) | commit된 command와 semantic event를 하나의 순서로 보존하는 in-memory 기록, 공유 lock과 저장 구조를 숨기는 sequence 기반의 제한된 Transcript 읽기 | 실행 중 capture 지점은 `runtime`, durable byte는 `session_repository` |
| [`session_repository`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/session_repository/mod.rs) | 저장 형식에 독립적인 append·suffix 읽기 계약, snapshot 복구 gate, typed storage pressure, 첫 single-writer versioned-JSONL 로컬 구현. rollback이 불확실한 append는 내구 pending marker로 격리 | semantic payload를 맡을 향후 Journal codec과 runtime owner, 지속적인 frontend 알림. 현재 synchronous Rust trait은 local 조립 seam이며 고정된 remote transport 계약이 아니고, 아직 실행 중인 Session에는 연결되지 않음 |
| [`runtime`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/runtime/mod.rs) | backend 수락, semantic commit, Journal capture 순서, backend 관찰 결과 변환, 실패 시 활성 작업 종료 | provider port는 `backend/contract.rs` |
| [`agent_session`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/mod.rs) | frontend를 막지 않는 접근, 크기가 제한된 command lane, 용량 1의 Journal 변경 알림, worker 소유권, 시작 취소, 종료 조율 | worker가 소유한 의미 처리는 `runtime` |
| [`backend/contract.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/contract.rs) | provider capability, command, semantic event, polling, 취소, failure kind, 명시적 정리 | 구체적인 adapter |
| [`backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs) | `codex app-server` 생명주기, JSON transport와 protocol 분류, provider ID 연결, core event로 변환 | 새 provider 동작을 노출하기 전 `backend/contract.rs` |

`AgentBackend`가 현재 provider 교체 지점이며 Codex wire value는
`backend/codex` 아래에 있다.
[command와 event 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.command-event-boundary.md)와
[Codex app-server adapter](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md)가
각 동작 제약을 소유한다.
[Session Journal](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.observability.session-journal.md)은
replay source 계약을 소유한다. 현재 코드는 semantic record만 메모리에
capture하고 구체적인 `TranscriptReader`로 공개한다. 별도의
`SessionRepository`가 durable opaque record를 제공하지만, 실행 중인
runtime은 아직 이 저장소에 쓰지 않는다. 따라서 현재 Session을 재개할 수
있거나 backend exchange까지 기록한다고 주장하지 않는다. 제품 범위를
바꾸기 전에 semantic codec과 runtime 소유권을 추가하고, 실제 remote
reader가 생길 때 local·remote reader 공통 인터페이스를 추출한다.

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

`runner::TuiSession`은 한 번의 터미널 소유 기간보다 오래 유지할 수 있는
대화 기록, editor, 대기 중인 요청, view, backpressure로 전달되지 못한
agent dispatch 상태를 소유한다. 보존된 상태에는 해당 agent Session의
식별자가 있으므로 재진입할 때도 같은 agent 연결을 유지한다.
`runner/unix.rs`는 매 터미널 소유 기간마다 터미널 입력, presenter,
viewport 소유권, frame 이력을 새로 얻으며, 이 자원들은 `TuiSession`으로
옮기지 않는다. 정리가 성공한 `Ctrl+Z`는 이 세대 전용 자원을 모두 복원한
뒤에만 `TerminalOutcome::SuspendRequested`를 반환한다.

`surface`는 공통으로 완성된 상태다. 터미널과 HTML Projection은 이를
각자 소비하며, 어느 쪽도 다른 쪽의 layout 의미를 정의하지 않는다.

## 저장소 개발 도구

[`tools/xtask`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/xtask/src/lib.rs)는
`yo` 제품이 아니라 이 저장소를 관리하는 구조화된 검사를 소유한다. 변경
경로와 commit trailer를 분류하여 Slice 검수와 Developer Docs 영향을
확인하고, Rust test에 이해 가능한 인접 설명이 있는지도 검사한다.
`hk.pkl`은 언제 실행할지를 결정하고, `xtask`는 규칙의 구현과 test를
담당한다. Methexis와 Librarian은 각자의 지식 domain 책임을 유지하며,
단순한 외부 명령 조합은 `hk` 또는 작은 검증 script에 남는다.

소유자를 골랐다면 [검증](../validation/)을 변경 경계에서 증거로
이어지는 단일 지도로 사용한다. 실제 터미널 동작이 관련되면
[터미널 환경 매트릭스](../validation/terminal-matrix.md)를 따른다.
[Slice](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#slice-contract)를
닫기 전에는 변경이 실제로 지나간 경계에 대해서만 검사를 넓힌다.
