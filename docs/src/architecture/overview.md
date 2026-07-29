# Architecture

`yo` currently has one frontend-independent core, one TUI library, and one
process entry point.

| Area | Owns | Start here |
|---|---|---|
| `yo-cli` | Process entry, Unix termination coordination, and selecting Inline or Fullscreen presentation | [`crates/yo-cli/src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs) |
| `yo-core` | Agent session semantics, commands and events, backend ports, and the Codex app-server adapter | [`crates/yo-core/src/lib.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/lib.rs) |
| `yo-tui` | Input editing, transcript layout, terminal modes, rendering, and the shared HTML projection | [`crates/yo-tui/src/lib.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/lib.rs) |

The dependency direction is:

```text
yo-cli
├── yo-core
└── yo-tui
    └── yo-core
```

`yo-core` does not depend on a frontend. A future GUI can consume the same
agent session boundary without owning terminal policy.

## Contract owners

- [Frontend-independent core boundary](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.core.frontend-independent-boundary.md)
- [TUI-only crate boundary](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.crate.ui-only-boundary.md)
- [Module boundary policy](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.architecture.module-boundaries.md)

Continue to [Runtime flow](./runtime-flow.md) for the path followed by one user
turn.
