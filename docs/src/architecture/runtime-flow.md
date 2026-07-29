# Runtime flow

One interactive turn crosses the repository in this order:

```text
terminal input
    ↓
yo-tui input editor and runner
    ↓ semantic command
yo-core AgentSession
    ↓ backend request
Codex app-server adapter
    ↓ semantic runtime events
yo-tui transcript state and renderer
    ↓
Inline or Fullscreen terminal presentation
```

The boundaries intentionally exchange typed commands and events. Terminal
backend event types do not enter `yo-core`, and Codex wire messages do not
enter `yo-tui`.

Useful starting points:

- TUI runner: [`crates/yo-tui/src/runner/unix.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/unix.rs)
- Agent session: [`crates/yo-core/src/agent_session/mod.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/mod.rs)
- Codex adapter: [`crates/yo-core/src/backend/codex/mod.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs)
- Transcript state: [`crates/yo-tui/src/transcript/mod.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/transcript/mod.rs)
- Terminal presentation: [`crates/yo-tui/src/terminal/mode/mod.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/mode/mod.rs)

Relevant contracts:

- [Command and event boundary](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.command-event-boundary.md)
- [Session, turn, and activity semantics](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.session-turn-activity.md)
- [Typed TUI flow](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.typed-flow.md)
