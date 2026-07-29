# Find the change

Start from the observable outcome, not a familiar filename. This page selects
the first owner and the next boundary to inspect. Use the [Code map](../architecture/code-map.md)
for module responsibilities, [Runtime flow](../architecture/runtime-flow.md)
for cross-crate ordering, and [Validation](../validation/README.md) for the
meaning of the resulting evidence.

Methexis owns behavioral contracts. The routes below link to those owners
instead of redefining them.

## Choose the search depth

After selecting a boundary, use the least expensive search that answers the
question:

| Question | Use | Why |
|---|---|---|
| Where is this exact type, function, error text, or event variant named? | `rg` | Fast literal or regular-expression search with no indexing step |
| What defines this symbol, which references resolve to it, or what survives aliases and macro expansion? | [rust-analyzer definition and reference navigation](https://rust-analyzer.github.io/book/features.html) | Uses Rust project semantics rather than textual coincidence |
| Where does this syntax shape occur despite different names or formatting? | [ast-grep read-only structural search](https://astgrep.com/reference/cli.html) | Matches parsed Rust nodes with code-like patterns and metavariables |
| Which repository responsibility should contain the result? | This page and the [Code map](../architecture/code-map.md) | AST shape and symbol resolution do not decide architectural ownership |

`rust-analyzer` is already selected in `rust-toolchain.toml`; use its editor/LSP
definition and reference operations for semantic navigation. Its CLI
subcommands are not the repository interface because rust-analyzer documents
them as unstable.

ast-grep is a candidate pilot, not a required repository tool. A useful
read-only trial would search trait implementations independent of concrete type
names:

```bash
ast-grep run --lang rust \
  --pattern 'impl $TRAIT for $TYPE { $$$BODY }' crates
```

It can also produce a syntax-oriented outline:

```bash
ast-grep outline --lang rust crates/yo-core/src
```

Structural matches understand parsed syntax, not Rust type resolution or macro
semantics; confirm an important result with rust-analyzer and the owning tests.
Do not use structural rewrite during the navigation pilot. If repeated trials
show that a query saves exploration time without noisy matches, adopt a pinned
version and checked-in query in a separate Slice. Use the explicit
`ast-grep` executable name rather than the `sg` alias, which can refer to an
unrelated system command.

## Choose the first owner

| Desired outcome | Start here | Continue only when | Read and run |
|---|---|---|---|
| Change decoded keys, editing, paste, configurable bindings, or exit gestures | [`yo-tui/src/input`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/input/mod.rs) | Visible cursor measurement changes: `prompt`; a semantic agent action changes: `runner` | [Active-Turn input](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.active-turn-input.md); `cargo test -p yo-tui input::` |
| Change prompt wrapping, cursor visibility, or prompt viewport behavior | [`yo-tui/src/prompt`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/prompt/mod.rs) | Region allocation changes: `shell` or `layout`; edit semantics change: `input` | [Bounded view](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.bounded-view.md); `cargo test -p yo-tui prompt::` |
| Change transcript items, streaming updates, scrolling, or page movement | [`yo-tui/src/transcript`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/transcript/mod.rs) | Transcript/prompt allocation changes: `shell`; event interpretation changes: `runner/state.rs` | [Typed TUI flow](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.typed-flow.md); `cargo test -p yo-tui transcript::` |
| Change shell regions or completed frame composition | [`yo-tui/src/shell`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/shell/mod.rs) and [`layout`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/layout/mod.rs) | Cell writes or clipping change: `surface`; terminal effects change: `terminal` | [Surface geometry](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.geometry.md); `cargo test -p yo-tui` |
| Change grapheme width, cell ownership, clipping, resolved style, or diff spans | [`yo-tui/src/surface`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/surface/mod.rs) and [`text`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/text/mod.rs) | A projection disagrees: `terminal` or `html`; composition policy changes: the calling component | [Surface model](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.model-ownership.md); `cargo test -p yo-tui` |
| Change HTML emitted from a completed frame | [`yo-tui/src/html`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/html/mod.rs) | Terminal and HTML disagree on common state: `surface`; only HTML encoding differs: remain here | [HTML projection](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.html-projection.md); `cargo test -p yo-tui --test rendering_parity` |
| Change ANSI encoding or typed terminal operations | [`yo-tui/src/terminal`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/mod.rs) | Screen policy changes: `terminal/mode`; OS effects change: `terminal/backend` | [Terminal operations](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.terminal-ops.md); `cargo test -p yo-tui terminal::` |
| Change Inline or Fullscreen presentation, viewport updates, or restoration | [`yo-tui/src/terminal/mode`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/mode/mod.rs) | Unix acquisition or output changes: `terminal/backend`; process signal behavior changes: `yo-cli/process` | [Lifecycle restoration](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.lifecycle-restoration.md); `cargo test -p yo-tui terminal::mode::` |
| Change live-loop input/event ordering, backpressure handling, or TUI event projection | [`yo-tui/src/runner`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/mod.rs) | Session admission or event meaning changes: `yo-core/agent_session`; terminal policy changes: `terminal/mode` | [Typed TUI flow](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.typed-flow.md); `cargo test -p yo-tui runner::` |
| Change Session, Turn, Activity, request, command, or event meaning | [`yo-core/src/engine`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/engine/mod.rs), [`command.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/command.rs), or [`event.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/event.rs) | Provider acceptance or observations are involved: `runtime`; frontend concurrency is involved: `agent_session` | [Session lifecycle](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.session-turn-activity.md); `cargo test -p yo-core` |
| Change frontend admission, backpressure, worker ownership, startup cancellation, or shutdown | [`yo-core/src/agent_session`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/mod.rs) | Semantic transition changes: `engine` or `runtime`; TUI gesture changes: return to `yo-tui/runner` | [Command and event boundary](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.command-event-boundary.md); `cargo test -p yo-core agent_session::tests` |
| Change the provider-neutral backend port or command acceptance ordering | [`yo-core/src/backend/contract.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/contract.rs) and [`runtime`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/runtime/mod.rs) | Codex-specific wire behavior changes: `backend/codex`; public frontend use changes: `lib.rs` and `agent_session` | [Frontend-independent core](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.core.frontend-independent-boundary.md); `cargo test -p yo-core` |
| Change Codex process, JSON protocol, version gate, ID correlation, or event translation | [`yo-core/src/backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs) | Provider-neutral meaning changes: `backend/contract`, `runtime`, or `engine` | [Codex adapter](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md); `cargo test -p yo-core backend::codex::tests` |
| Change CLI arguments, working-directory capture, startup order, or top-level failure aggregation | [`yo-cli/src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs) | Agent adaptation changes: `agent`; signal policy changes: `process/termination` | [Module and host boundaries](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.architecture.module-boundaries.md); `cargo test -p yo-cli` |
| Change Unix termination observation, signal priority, disposition, or restoration | [`yo-cli/src/process/termination`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/termination/mod.rs) | Terminal restoration changes: `yo-tui/terminal/mode`; typed observation changes: `yo-tui/runner` | [Process termination coordinator](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.process-termination-coordinator.md); `cargo test -p yo-cli process::termination::tests` |

The first check narrows feedback while developing. Run the Slice-close baseline
and any affected environment matrix before accepting the change.

## Follow a symptom across boundaries

The visible failure is not always owned by the module that displays it:

| Symptom | Trace in this order |
|---|---|
| Submitted text appears, but no Turn starts | `yo-tui/runner/state.rs` → `yo-cli/agent` → `yo-core/agent_session/admission.rs` → worker/runtime → backend |
| Codex accepted work, but the transcript does not update | `backend/codex/events.rs` → `AgentRuntime::poll_event` → agent-session event lane → `TuiState::observe` → transcript |
| Input stalls only while the backend is busy | runner pending dispatch → `AgentSession::dispatch`/`retry` → bounded command lane → worker lifecycle |
| Terminal state is damaged after a normal exit | terminal mode guard → presenter cleanup → Unix backend; inspect process termination only when a signal path is involved |
| A signal exits before cleanup is visible | TUI typed termination observation → guarded terminal return → agent shutdown → `TerminationCoordinator::with_active_session` |
| Terminal and HTML disagree | shared fixture and completed `Surface` → terminal projection and HTML projection independently |
| Only tmux, SSH, or nested tmux fails | real environment route first; then terminal mode/backend only after the failing route is reproduced |

For the full order of startup, one Turn, and cleanup, use the
[Runtime flow](../architecture/runtime-flow.md). For what a passed, failed, or
unverified result means, use [Validation](../validation/README.md).

## Keep the change in its owner

Before widening the edit:

1. Confirm whether the outcome changes behavior or only an implementation
   detail.
2. Read the linked Methexis KnowledgeUnit when behavior changes.
3. Keep Codex JSON inside `backend/codex` and terminal types inside `yo-tui`.
4. When the required behavior belongs to another owner, move the change there
   instead of adding a cross-layer shortcut.
5. Add the smallest discriminating test beside the owner, then widen validation
   across every boundary the change actually crosses.

If the intended outcome does not fit a row without changing more than one
public boundary, inspect the [Code map](../architecture/code-map.md) before
editing. That is usually a design decision or a Slice split, not a reason to
let one module absorb another owner's responsibility.
