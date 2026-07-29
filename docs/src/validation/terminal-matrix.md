# Terminal environment matrix

Real PTY output is the authority for terminal behavior. HTML fixtures help
diagnosis and parity review but do not replace these checks.

## What the normal test set covers

On Linux, the non-ignored `yo-cli` tests create real PTYs and exercise normal
Fullscreen exit plus signal-driven restoration. They do not require tmux,
`sshd`, or an installed Codex:

```bash
cargo test -p yo-cli pty_tests::
```

The process-coordinator tests separately exercise handler installation,
rollback, shutdown compensation, thread ownership, and isolated subprocess
signal behavior:

```bash
cargo test -p yo-cli process::termination::tests
```

These host-integrated checks are part of the ordinary package test run. Their
passing result does not imply that tmux or SSH behavior ran.

## Installed Codex checks

Verify the stdio initialize and shutdown boundary without a model turn:

```bash
cargo test -p yo-core local_codex_initializes_and_shuts_down \
  -- --ignored --nocapture --test-threads=1
```

Verify one authenticated model turn, tool execution, file change, semantic
events, and explicit cleanup in a disposable workspace:

```bash
cargo test -p yo-core local_codex_completes_a_real_file_change \
  -- --ignored --nocapture --test-threads=1
```

The second check performs an external model operation. Its Turn wait is capped
at 180 seconds; total runtime also includes Codex startup and shutdown. Run it
only where compatible Codex authentication and writable Codex state are
available.

## Linux tmux and SSH checks

Local tmux, both presentation modes:

```bash
cargo test -p yo-cli --test terminal_matrix local_tmux_ \
  -- --ignored --nocapture --test-threads=1
```

SSH and tmux inside SSH, both presentation modes:

```bash
cargo test -p yo-cli --test terminal_matrix ssh:: \
  -- --ignored --nocapture --test-threads=1
```

The local tmux tests require compatible installed `tmux` and Codex. The SSH
tests start an isolated localhost `sshd`, generate temporary keys, and remove
their fixture directory. They require compatible local `ssh`, `sshd`,
`ssh-keygen`, Codex, a set `USER` naming the local SSH account, and, for the
nested cases, tmux.

These tests fail when a required command or assertion is unavailable; they do
not convert a missing environment into a successful skip.

## Platform coverage

The executable environment matrix currently covers:

| Host and route | Inline | Fullscreen | Evidence |
|---|---:|---:|---|
| Linux direct real PTY | Unverified | Yes | Normal `yo-cli` tests currently cover Fullscreen |
| Linux local tmux | Yes | Yes | Ignored environment tests |
| Linux SSH | Yes | Yes | Ignored environment tests |
| Linux tmux inside SSH | Yes | Yes | Ignored environment tests |
| macOS compile | — | — | `cargo check` on a real macOS CI host |
| macOS terminal behavior | Unverified | Unverified | No real-host environment run yet |

`tools/validation/yo-cli-unix-matrix.sh` checks all `yo-cli` targets on the
current Unix host and reports the other host as unverified. The CI workflow
runs the equivalent compile check independently on Linux and macOS; compilation
does not replace terminal behavior evidence.

## Reporting a matrix run

Keep the result small but explicit:

```text
Host:
Route and mode:
Command:
Result: passed | failed | unverified
Observed failure or missing prerequisite:
```

Do not infer one route from another. A passing local tmux run does not mark SSH,
nested tmux, or macOS as verified.

Contract: [Rendering validation authority](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.validation-matrix.md)
