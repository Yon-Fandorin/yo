# Validation

Choose evidence by the boundary that changed. Start with the smallest check
that can distinguish the expected behavior from its important failure, then
widen before closing the
[Slice](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#slice-contract).

## Evidence layers

| Layer | What it establishes | Examples |
|---|---|---|
| In-process | Deterministic state, protocol, layout, rendering, and injected failure behavior | `yo-core` engine/runtime tests; `yo-tui` component tests; rendering parity goldens |
| Host-integrated | Behavior of real host facilities without optional installed services | Linux PTY, termios, process signal, and terminal-restoration tests in `yo-cli` |
| External environment | Compatibility with installed programs, authentication, and nested terminal environments | Codex, tmux, local `sshd`, SSH, and tmux inside SSH |

The first layer gives fast diagnosis but cannot prove an OS terminal lifecycle.
The host-integrated layer exercises real Unix boundaries but cannot prove every
terminal multiplexer or remote session. The external layer closes those gaps
only for the environment where it actually ran.

An ignored or unavailable environment check is **unverified**, not passed.
Record the missing command, host, credential, or platform instead of weakening
the assertion or silently skipping it.

## Start from the changed boundary

| Change area | First useful command | Closest evidence |
|---|---|---|
| Session, Turn, Activity, engine, or runtime semantics | `cargo test -p yo-core` | `crates/yo-core/src/tests` and the owning module tests |
| Agent-session admission, concurrency, startup, or shutdown | `cargo test -p yo-core agent_session::tests` | `crates/yo-core/src/agent_session/tests` |
| Codex protocol translation or provider-ID correlation | `cargo test -p yo-core backend::codex::tests` | `crates/yo-core/src/backend/codex/tests.rs` |
| Decoded input, editing, paste, bindings, or exit gestures | `cargo test -p yo-tui input::` | Tests beside `yo-tui/src/input` |
| Prompt wrapping, cursor visibility, or viewport behavior | `cargo test -p yo-tui prompt::` | Tests beside `yo-tui/src/prompt` |
| `@` trigger, stale result, selection replacement, local ranking, or Git-ignore discovery | `cargo test -p yo-tui workspace_reference` and `cargo test -p yo-core workspace_reference` | `yo-tui/src/prompt/workspace_reference.rs` and `yo-core/src/workspace_reference` |
| Transcript items, streaming revisions, or scrolling | `cargo test -p yo-tui transcript::` | Tests beside `yo-tui/src/transcript` |
| Shell composition, layout, Surface, Unicode width, or text flow | `cargo test -p yo-tui` | Tests beside the owning `yo-tui` module |
| ANSI operations or presentation-mode policy | `cargo test -p yo-tui terminal::` | Tests under `yo-tui/src/terminal` |
| Inline or Fullscreen mode behavior | `cargo test -p yo-tui terminal::mode::` | Tests under `yo-tui/src/terminal/mode` |
| Live-loop ordering, backpressure, or event projection | `cargo test -p yo-tui runner::` | Tests under `yo-tui/src/runner` |
| Terminal and HTML projection of the same completed frame | `cargo test -p yo-tui --test rendering_parity` | `crates/yo-tui/tests/rendering_parity` and its goldens |
| Process termination or real terminal restoration | `cargo test -p yo-cli pty_tests::` | `crates/yo-cli/src/pty_tests.rs` |
| Unix process-coordinator state and compensation | `cargo test -p yo-cli process::termination::tests` | `crates/yo-cli/src/process/termination/tests` |
| Required explanations immediately above Rust tests | `cargo xtask check test-explanations` | Rust sources under `crates/` and `tools/` |
| Slice changes remain inside their bound local write-set | `cargo xtask check slice-scope` | One active Slice worktree; the planner first runs `cargo xtask slice-contract bind <contract.json>` |
| Two Slice contracts have a common current integration base and disjoint declared ownership | `cargo xtask check slice-parallel <left.json> <right.json>` | Direct Slices use `develop`; Wave Slices use their Wave branch |
| Repository hook policy or structured development checks | `cargo test -p xtask` | `tools/xtask/src` |
| Linux/macOS conditional compilation | `bash tools/validation/yo-cli-unix-matrix.sh` | Local host result plus `.github/workflows/unix-compile.yml` for both hosts |
| tmux, SSH, or nested tmux behavior | See the [terminal environment matrix](./terminal-matrix.md) | Ignored `yo-cli` environment tests |

These commands are entry points, not permission to ignore affected neighboring
boundaries. For example, an edit to `AgentSession` can require both its focused
tests and the TUI runner tests when the admission result observed by the
frontend changes.

## Reading a result

- **Passed** means the named command ran its assertions successfully in the
  stated environment.
- **Failed** means the command ran and found a mismatch, timeout, panic, or
  cleanup error. Follow the first owning boundary, then retain any additional
  cleanup failures.
- **Unverified** means the check did not run in the required environment. Keep
  it visible as a coverage gap.

Goldens and snapshots establish an exact projection of their fixture. Review
the diff when intentionally updating one; do not treat regeneration alone as
evidence that the new output is correct.

## Slice-close baseline

After focused checks pass, run the repository baseline:

```bash
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
hk check
```

`cargo test` runs the normal test set and compiles ignored tests; it does not
execute ignored environment tests. `hk check` selects repository checks from
`hk.pkl` according to the changed paths, including formatting, test
explanations, affected crate checks, Methexis checks, and Developer Docs checks.
Installation and hook usage belong to
[`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#local-checks).

Use focused checks from the local Slice contract while editing, then run this
Slice-close baseline once the outcome is complete. For the exact staged
Methexis activation interval, `hk` uses prospective validation and defers the
ordinary Methexis tests; immediately after integration, run the ordinary full
Methexis check and tests against trusted `develop`.

If the Slice changes a platform or external-environment boundary, add the
relevant matrix command rather than claiming the baseline covered it.

## Useful owners

- Hook selection: [`hk.pkl`](https://github.com/Yon-Fandorin/yo/blob/develop/hk.pkl)
- Structured repository checks: [`tools/xtask`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/xtask/src/lib.rs)
- Unix host compile check: [`tools/validation/yo-cli-unix-matrix.sh`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/validation/yo-cli-unix-matrix.sh)
- Rendering parity fixture: [`crates/yo-tui/tests/fixtures/rendering-parity/README.md`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/tests/fixtures/rendering-parity/README.md)
- Test explanation policy: [`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#test-code)
