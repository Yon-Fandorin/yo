# Find the change

Start from the observable outcome, then follow the listed path. Read the linked
Methexis contract before changing behavior rather than implementation detail.

| Desired outcome | Start | Continue through | Contract |
|---|---|---|---|
| Change text entry or key behavior | `yo-tui/src/input` | `prompt` → `runner` | [Active-turn input](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.active-turn-input.md) |
| Change transcript content or scrolling | `yo-tui/src/transcript` | `shell` → `runner` | [Typed TUI flow](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.typed-flow.md) |
| Change Inline or Fullscreen rendering | `yo-tui/src/terminal/mode` | `terminal/ops.rs` → `surface` | [Mode selection](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.mode-selection.md) |
| Change terminal acquisition or restoration | `yo-tui/src/terminal/backend` | `terminal/mode` → `yo-cli/src/process` | [Lifecycle restoration](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.lifecycle-restoration.md) |
| Change session or turn semantics | `yo-core/src/agent_session` | `engine` → `runtime` | [Session semantics](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.session-turn-activity.md) |
| Change Codex integration | `yo-core/src/backend/codex` | `backend` → `agent_session` | [Codex adapter](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md) |

Paths in this table identify responsibility boundaries, not a complete file
inventory. Use code search inside the selected boundary for the concrete type
or operation.
