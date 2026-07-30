# Code map

Use this map to choose an ownership boundary before searching for a concrete
type or function. It describes stable responsibilities and entry points, not
every source file.

## Cross-crate route

The process host creates the provider and frontend, then connects them through
frontend-independent session semantics:

```text
yo-cli main
├── yo-core CodexBackend
├── yo-cli TuiAgentConnection
└── yo-tui runner
        ↕ AgentConnection
    yo-core AgentSession
        ↕ bounded command lane + coalesced Journal-change lane
    worker-owned AgentRuntime
        ├── AgentEngine
        └── AgentBackend
```

The current implementation seams are:

- process policy and cleanup order live in `yo-cli`;
- Session, Turn, Activity, command, and event meaning live in `yo-core`; and
- terminal interaction and presentation live in `yo-tui`.

The accepted responsibilities and future-GUI constraint remain owned by the
[frontend-independent core boundary](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.core.frontend-independent-boundary.md),
[module and host boundaries](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.architecture.module-boundaries.md),
and [UI-only crate boundary](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.crate.ui-only-boundary.md).

## yo-cli: process host

| Boundary | Owns | Does not own |
|---|---|---|
| [`src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs) | Argument parsing, presentation selection before terminal acquisition, working-directory capture, provider startup, terminal-generation reentry, and top-level cleanup aggregation | Agent semantics or terminal rendering |
| [`src/agent/mod.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/agent/mod.rs) | Adapting `yo-core::AgentSession` to the TUI's `AgentConnection` port, including the concrete local Transcript cursor | Provider protocol translation or a premature local/remote reader trait |
| [`src/process/job_control.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/job_control.rs) | Transactionally applying default `SIGTSTP`, suspending the process, and restoring inherited signal state after `SIGCONT` | TUI state or terminal restoration |
| [`src/process/termination`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/termination/mod.rs) | Unix signal installation, observation, restoration, and final disposition | Terminal state restoration |

Follow process-start or shutdown failures from `main.rs` into the owner named
in the error context. The signal coordinator lives in `yo-cli`; the TUI
receives only the typed, signal-neutral `TerminationEvent`. The
[process termination coordinator](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.process-termination-coordinator.md)
owns that contract.

## yo-core: agent semantics

[`src/lib.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/lib.rs)
is the public facade. Start there when a GUI, TUI, or provider adapter needs a
new shared capability.

| Module | Owns | Follow next |
|---|---|---|
| [`command.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/command.rs), [`event.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/event.rs), [`session.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/session.rs) | Provider-neutral commands, observable events, outcomes, and typed identities | `engine` for legal state transitions |
| [`engine`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/engine/mod.rs) | Deterministic Session, Turn, Activity, and request state transitions | `runtime` when a transition also crosses a provider boundary |
| [`journal`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/journal/mod.rs) | One ordered in-memory record of committed commands and semantic events; bounded sequence-based Transcript reads that hide the shared lock and storage layout | `runtime` for the live capture point; `session_repository` for durable bytes |
| [`session_repository`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/session_repository/mod.rs) | Storage-neutral append and suffix-read contract, snapshot recovery gate, typed storage pressure, and the first single-writer local versioned-JSONL implementation; a durable pending marker quarantines an append whose rollback is uncertain | A future Journal codec and runtime owner for semantic payloads and persistent frontend notification; the current synchronous Rust trait is a local composition seam, not a frozen remote transport contract, and is not wired into live Sessions yet |
| [`runtime`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/runtime/mod.rs) | Ordering backend acceptance, semantic commit, and Journal capture; translating backend observations; closing active work on failure | `backend/contract.rs` for the provider port |
| [`agent_session`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/mod.rs) | Nonblocking frontend access, bounded command lanes, a capacity-one Journal-change notification, worker ownership, startup cancellation, and shutdown coordination | `runtime` for worker-owned semantics |
| [`backend/contract.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/contract.rs) | Provider capabilities, commands, semantic events, polling, cancellation, failure kinds, and explicit cleanup | A concrete adapter |
| [`backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs) | `codex app-server` lifecycle, JSON transport and protocol classification, provider-ID correlation, and translation into core events | `backend/contract.rs` before exposing new provider behavior |

`AgentBackend` is the current provider seam, and Codex wire values live under
`backend/codex`. The
[command and event boundary](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.command-event-boundary.md)
and [Codex app-server adapter](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md)
own the corresponding behavioral constraints. The
[Session Journal](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.observability.session-journal.md)
owns the replay-source contract; current code captures only semantic records in
memory and exposes them through a concrete `TranscriptReader`. The separate
`SessionRepository` now provides durable opaque records, but no live runtime
path writes to it yet. It therefore does not make current Sessions resumable
or claim backend-exchange coverage. Add the semantic codec and runtime
ownership before changing that product claim; extract a local/remote reader
interface only when a real remote reader exists.

## yo-tui: terminal frontend

[`src/lib.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/lib.rs)
keeps the live runner facade narrow while exposing reusable completed-surface,
terminal-operation, and HTML-projection types.

| Module | Owns | Follow next |
|---|---|---|
| [`runner`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/mod.rs) | Public live-session facade, single terminal-owning loop, input/event orchestration, and final cleanup reporting | `runner/state.rs` for semantic UI transitions; `runner/unix.rs` for live orchestration |
| [`appearance`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/appearance/mod.rs) | Session-owned immutable appearance snapshots, monotonic revisions, resolved style roles, and explicit Rich/ASCII glyph profiles | `runner/session.rs` for ownership; `runner/state.rs` for frame pinning |
| [`input`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/input/mod.rs) | Decoded semantic key events, edit buffer, configurable bindings, exit gestures, and prompt editing | `prompt` for visible cursor layout |
| [`transcript`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/transcript/mod.rs) | Ordered user and agent items, streaming revisions, transcript layout, and scrolling state | `shell` for composition with the prompt |
| [`prompt`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/prompt/mod.rs) | Measuring and painting editor content plus cursor visibility | `input/editor` for edit semantics |
| [`shell`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/shell/mod.rs), [`layout`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/layout/mod.rs) | Allocating transcript and prompt regions, composing one completed frame, and reporting its cursor | `surface` for cell writes |
| [`surface`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/surface/mod.rs), [`text`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/text/mod.rs) | Adapter-independent cell state, Unicode graphemes and width, bounded views, diff spans, and terminal-independent text flow | `terminal` or `html` for projection |
| [`terminal`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/mod.rs) | Typed terminal operations and ANSI encoding | `terminal/mode` for presentation policy; `terminal/backend` for Unix effects |
| [`terminal/mode`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/mode/mod.rs), [`terminal/backend`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/backend/mod.rs) | Shared transactional restoration, Inline and Fullscreen presenters, panic routing, and the crate-private platform boundary | `yo-cli/process` only when process signal policy changes |
| [`html`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/html/mod.rs) | Deterministic browser projection of completed `Surface` state | `surface` when terminal and browser output disagree |

`runner::TuiSession` owns transcript, editor, pending-request, view,
backpressured agent-dispatch state, and one committed appearance snapshot that
can outlive one terminal ownership generation. Each redraw pins the appearance
revision before measurement and uses that same resolved snapshot through paint
and the completed `Surface`; plain session output pins the same session-owned
configuration. Reentry keeps the same agent connection because the retained
state contains identities from that agent Session. `runner/unix.rs` acquires
fresh terminal input, presenter, viewport ownership, and frame history for each
generation; those resources never move into `TuiSession`. A clean `Ctrl+Z`
returns `TerminalOutcome::SuspendRequested` only after those generation-local
resources are restored.

The `surface` is the common completed state. Terminal and HTML projections
consume it independently; neither projection defines layout meaning for the
other.

## Repository development tools

[`tools/xtask`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/xtask/src/lib.rs)
owns structured checks that maintain this repository rather than the `yo`
product. Its checks classify changed paths and commit trailers for Slice review
and Developer Docs impact, and verify that Rust tests carry nearby explanatory
comments. `hk.pkl` decides when to run them; `xtask` implements and tests their
rules. Methexis and Librarian retain their separate knowledge-domain
responsibilities, while simple external-command orchestration remains in `hk`
or a small validation script.

After choosing an owner, use [Validation](../validation/) as the
single map from changed boundary to evidence. Follow the
[terminal environment matrix](../validation/terminal-matrix.md) when real
terminal behavior is involved. Before closing a
[Slice](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#slice-contract),
widen checks only across the boundaries the change actually crosses.
