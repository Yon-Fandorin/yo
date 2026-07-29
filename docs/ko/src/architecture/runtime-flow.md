# 실행 흐름

변경이 크레이트 경계를 지나거나 오류 메시지만으로 소유자를 알기 어려울
때 이 흐름을 사용한다. 여기에는 현재 구현 경로가 담겨 있다. 각 경계가
어떤 의미여야 하는지는 계속 Methexis가 기준이다.

## 시작

프로세스 정책과 agent Session이 준비된 뒤에만 터미널을 획득한다.

```text
yo-cli
  표시 mode 해석과 cwd 확보
  TerminationCoordinator 설치
  CodexBackend transport 시작
      ↓
yo-core AgentSession
  worker 시작
  CreateSession
      ↓
Codex app-server
  initialize
  thread/start
      ↓
yo-core
  SessionCreated
      ↓
yo-tui
  터미널을 획득하고 Inline 또는 Fullscreen mode 진입
```

| 단계 | 현재 소유자 | 확인할 내용 |
|---|---|---|
| 1 | [`yo-cli/src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs) | `run`이 표시 mode를 선택하고 작업 디렉터리를 확보한 뒤 프로세스 종료 coordinator를 설치한다. |
| 2 | [`yo-core/backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs) | `CodexBackend::spawn`이 설정을 검증하고 stdio transport를 시작한다. provider handshake는 아직 하지 않는다. |
| 3 | [`yo-core/agent_session`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/mod.rs) | `AgentSession::start_cancellable`이 backend를 `yo-agent-runtime`이라는 worker thread로 넘긴다. 종료 관찰을 막지 않으면서 시작 완료를 기다린다. |
| 4 | [`yo-core/agent_session/worker.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/worker.rs) | `AgentWorker::initialize`가 `AgentRuntime`을 통해 `CreateSession`을 보낸다. |
| 5 | [`yo-core/backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs) | `CreateSession`이 `initialize`와 `thread/start`를 수행하고 semantic engine이 `SessionCreated`를 만든다. |
| 6 | [`yo-tui/runner/unix.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/unix.rs) | `run_with_mode`가 input과 터미널 상태를 획득하고 이미 선택된 표시 mode로 들어간다. |

handshake 중에 종료 요청이 오면 `AgentSession::start_inner`가 취소
callback을 관찰하고 backend 중지를 요청한 뒤 worker 정리를 기다린다.
그리고 TUI에 Session을 넘기지 않은 채 반환한다. 이 경우 터미널 mode
코드가 아니라 여기서 조사를 시작한다.

## 활성 Turn 하나

제출된 prompt는 다음 경로를 지난다.

```text
terminal input
    ↓
TuiState::handle
    ↓ AgentIntent::Submit
TuiAgentConnection
    ↓
AgentSession admission and bounded command lane
    ↓
AgentWorker
    ↓ AgentCommand::StartTurn or SteerTurn
AgentRuntime
    ├── AgentEngine으로 검증
    ├── AgentBackend를 통해 수락
    └── AgentEngine으로 commit
          ↓
Codex app-server adapter
    ↓ BackendEvent
AgentRuntime
    ↓ AgentEvent
bounded event lane
    ↓
TuiState::observe → transcript → completed Surface
    ↓
Inline 또는 Fullscreen presenter
```

조사할 때 유용한 지점은 다음과 같다.

1. [`TuiState::handle`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/state.rs)는
   사용자가 제출한 text를 기록하고 frontend에 독립적인
   `AgentIntent::Submit`을 만든다.
2. [`TuiAgentConnection`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/agent/mod.rs)은
   좁은 adapter다. Session이나 provider 의미를 소유하지 않고 dispatch,
   retry, poll 연산을 전달한다.
3. [`agent_session/admission.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/admission.rs)는
   Submit을 `StartTurn` 또는 `SteerTurn`으로 결정한다. state lock이
   사용 중이거나 크기가 제한된 lane이 가득 찼다면, TUI loop가 다시
   시도할 수 있도록 내부가 드러나지 않는 pending command를 반환한다.
4. [`AgentWorker`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/worker.rs)만
   runtime을 실행하고 polling할 수 있다. 터미널을 소유한 thread는
   provider I/O를 기다리지 않는다.
5. [`AgentRuntime`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/runtime/mod.rs)은
   command 검증, backend 수락, semantic commit 순서를 보장한다.
   provider 관찰 결과도 semantic engine을 통해 변환한 뒤 `AgentEvent`로
   공개한다.
6. [`drain_agent`와 `redraw`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/unix.rs)는
   이미 도착한 semantic event를 소비하고 TUI 상태를 갱신한다. 완성된
   `Surface`를 조합해 활성 presenter로 보낸다.

Codex JSON과 provider identifier는 backend adapter 밖으로 나오지 않는다.
터미널 input event와 rendering type은 `yo-tui` 밖으로 나오지 않는다.
그 사이를 지나는 command와 event type은 `yo-core`가 소유한다.

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
yo_tui::run_with_mode 반환
    ↓
AgentSession::shutdown
  worker 중지 → backend 중지 → 활성 semantic work 종료
    ↓
TerminationCoordinator가 활성 session을 마무리
    ├── 사용자 종료: yo-cli로 반환
    └── signal: 선택된 signal의 기본 disposition 적용
          ↓
일반 반환에서는 yo-cli가 설치했던 signal 상태를 복원
```

TUI는 `UserRequested` 또는 `TerminationRequested`만 보고한다. 어떤
signal인지 식별하거나 프로세스의 마지막 동작을 선택하지 않는다. guard가
있는 runner는 어떤 결과든 반환하기 전에 터미널 상태를 복원한다.
`run_agent_session`은 터미널 연산이 실패했더라도 agent shutdown을
호출하고, 필요하면 두 실패를 모두 모아 보고한다.

일반 반환에서는
[`TerminationCoordinator::shutdown`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/termination/mod.rs)이
설치했던 signal disposition과 설치 thread의 원래 mask를 복원한다.
종료 signal이 선택되면 `with_active_session`은 TUI와 agent 정리 경로가
반환될 때까지 기다린다. 그 뒤 signal을 일반 애플리케이션 오류로 바꾸지
않고 해당 signal의 기본 disposition을 적용한다.

## 첫 소유자 찾기

처음 실패한 경계에 가장 가까운 오류 문맥부터 따라간다.

| 보이는 문맥 | 시작 지점 |
|---|---|
| `starting Codex` | transport 시작을 포함한 `yo-core/backend/codex` |
| `creating the agent Session` | `yo-core/agent_session` 시작과 worker handshake |
| `terminal session` | `yo-tui/runner`와 터미널 mode 정리 |
| `agent cleanup` | `yo-core/agent_session::shutdown`, 그다음 runtime/backend 정리 |
| `process termination session` 또는 `process termination cleanup` | `yo-cli/process/termination` |

뒤이어 발생한 정리 실패를 버리지 않는다. 현재 최상위 경로는 서로
독립적인 정리 경계를 모두 시도하고 각 오류 문맥을 함께 보고한다.

## 계약 소유자

- [command와 event 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.command-event-boundary.md)
- [Session, Turn, Activity 의미](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.session-turn-activity.md)
- [활성 Turn input](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.active-turn-input.md)
- [Codex app-server adapter](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md)
- [typed TUI 흐름](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.typed-flow.md)
- [표시 mode 선택](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.mode-selection.md)
- [터미널 생명주기 복원](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.lifecycle-restoration.md)
- [프로세스 종료 coordinator](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.process-termination-coordinator.md)

실패한 경계를 찾았다면 [검증](../validation/)에서 수정 결과를
확인할 증거를 선택한다.
