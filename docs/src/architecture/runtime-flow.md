# Runtime flow

Use these traces when a change crosses crate boundaries or when an error
message does not make its owner obvious. They describe the current
implementation path. Methexis remains the authority for what each boundary
must mean.

## Startup

The terminal is acquired only after process policy and the agent Session are
ready:

```text
yo-cli
  parse mode and capture cwd
  install TerminationCoordinator
  spawn CodexBackend transport
      ↓
yo-core AgentSession
  start worker
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
  acquire terminal and enter Inline or Fullscreen mode
```

| Step | Current owner | What to follow |
|---|---|---|
| 1 | [`yo-cli/src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs) | `run` selects the presentation mode, captures the working directory, and installs the process termination coordinator. |
| 2 | [`yo-core/backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs) | `CodexBackend::spawn` validates configuration and starts the stdio transport. It defers the provider handshake. |
| 3 | [`yo-core/agent_session`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/mod.rs) | `AgentSession::start_cancellable` transfers the backend to the worker thread (named `yo-agent-runtime`) and waits for startup without blocking termination observation. |
| 4 | [`yo-core/agent_session/worker.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/worker.rs) | `AgentWorker::initialize` sends `CreateSession` through `AgentRuntime`. |
| 5 | [`yo-core/backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs) | `CreateSession` performs `initialize` and `thread/start`; the semantic engine produces `SessionCreated`. |
| 6 | [`yo-tui/runner/unix.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/unix.rs) | `run_session_with_mode` acquires input and terminal state for the first terminal ownership generation, then enters the already selected presentation mode. |

If termination arrives during the handshake, `AgentSession::start_inner`
observes the cancellation callback, requests the backend stop, waits for worker
cleanup, and returns without giving the TUI a Session. Start investigation
there, not in terminal mode code.

## One active turn

A submitted prompt follows this route:

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
    ├── validate with AgentEngine
    ├── accept through AgentBackend
    └── commit with AgentEngine
          ↓
Codex app-server adapter
    ↓ BackendEvent
AgentRuntime
    ↓ AgentEvent
bounded event lane
    ↓
TuiState::observe → transcript → completed Surface
    ↓
Inline or Fullscreen presenter
```

The useful inspection points are:

1. [`TuiState::handle`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/state.rs)
   records the user's submitted text and emits the frontend-neutral
   `AgentIntent::Submit`.
2. [`TuiAgentConnection`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/agent/mod.rs)
   is a narrow adapter. It forwards dispatch, retry, and poll operations without
   owning Session or provider semantics.
3. [`agent_session/admission.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/admission.rs)
   resolves Submit to `StartTurn` or `SteerTurn`. A busy state lock or full
   bounded lane returns an opaque pending command for the TUI loop to retry.
4. [`AgentWorker`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/worker.rs)
   is the only owner that executes and polls the runtime. The terminal-owning
   thread does not wait on provider I/O.
5. [`AgentRuntime`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/runtime/mod.rs)
   orders command validation, backend acceptance, and semantic commit. It also
   translates a provider observation through the semantic engine before
   publishing an `AgentEvent`.
6. [`drain_agent` and `redraw`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/unix.rs)
   consume already available semantic events, update TUI state, compose a
   completed `Surface`, and send it to the active presenter.

Codex JSON and provider identifiers end at the backend adapter. Terminal input
events and rendering types end in `yo-tui`. The command and event types crossing
the middle are owned by `yo-core`.

## Suspend and resume

`Ctrl+Z` closes terminal ownership without closing the application Session:

```text
Ctrl+Z press
    ↓
TUI returns SuspendRequested after guarded terminal restoration
    ↓
TerminationCoordinator finalizes the active cleanup lease
    ├── termination selected: shut down the live agent and replay that signal
    └── no termination: return to Idle
          ↓
yo-cli applies default SIGTSTP and the process stops
          ↓ SIGCONT
restore inherited SIGTSTP state
          ↓
open a fresh active lease and terminal ownership generation
```

`TuiSession` and the same agent connection remain alive while the process is
stopped. Terminal input, raw mode, presenter, viewport ownership, and frame
history do not: each resumed generation reacquires them and starts with a full
frame. `process/job_control.rs` temporarily installs the default `SIGTSTP`
action and restores the inherited action and mask after continuation.

The process may suspend only after `with_active_resource` has finalized the
cleanup lease with no selected termination signal. If a configured termination
arrives at that boundary, its resource-cleanup callback shuts down the retained
agent and that exact signal wins over suspension.

Contract: [Terminal job-control suspend and resume](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.job-control-suspend-resume.md)

## Exit and cleanup

User exit and process termination share the same cleanup route until the
process host applies signal policy:

```text
exit gesture or typed TerminationEvent
    ↓
yo-tui loop returns its reason
    ↓
terminal guard restores presentation state
    ↓
yo_tui::run_session_with_mode returns Exited
    ↓
AgentSession::shutdown
  stop worker → stop backend → close active semantic work
    ↓
TerminationCoordinator finishes the active resource lease
    ├── user exit: return to yo-cli
    └── signal: apply the selected signal's default disposition
          ↓
yo-cli restores installed signal state on ordinary return
```

For a completed application Session, the TUI reports `UserRequested` or
`TerminationRequested`; it neither identifies signals nor chooses their final
process behavior. Its guarded runner restores terminal state before returning
either outcome. `run_agent_generation` then calls agent shutdown even when the
terminal operation failed and aggregates both failures when necessary.

On an ordinary return, [`TerminationCoordinator::shutdown`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/termination/mod.rs)
restores installed signal dispositions and the installing thread's original
mask. On a selected termination signal, `with_active_resource` waits until the
TUI cleanup route has returned, invokes retained-agent cleanup when necessary,
then applies that signal's default disposition instead of turning it into a
normal application error.

## Finding the first owner

Follow the context nearest the first failed boundary:

| Visible context | Start here |
|---|---|
| `starting Codex` | `yo-core/backend/codex`, including transport startup |
| `creating the agent Session` | `yo-core/agent_session` startup and worker handshake |
| `terminal session` | `yo-tui/runner` and terminal mode cleanup |
| `agent cleanup` | `yo-core/agent_session::shutdown`, then runtime/backend cleanup |
| `process termination session` or `process termination cleanup` | `yo-cli/process/termination` |
| `suspending the process` | `yo-cli/process/job_control` |

Do not discard later cleanup failures: the current top-level path attempts the
independent cleanup boundaries and reports their contexts together.

## Contract owners

- [Command and event boundary](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.command-event-boundary.md)
- [Session, Turn, and Activity semantics](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.session-turn-activity.md)
- [Active-Turn input](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.active-turn-input.md)
- [Codex app-server adapter](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md)
- [Typed TUI flow](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.typed-flow.md)
- [Presentation mode selection](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.mode-selection.md)
- [Terminal lifecycle restoration](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.lifecycle-restoration.md)
- [Process termination coordinator](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.process-termination-coordinator.md)
- [Terminal job-control suspend and resume](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.job-control-suspend-resume.md)

After locating the failing boundary, use [Validation](../validation/)
to choose the evidence that can confirm the fix.
