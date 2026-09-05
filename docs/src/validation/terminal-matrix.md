# Terminal environment matrix

Real PTY output is the authority for terminal behavior. HTML fixtures help
diagnosis and parity review but do not replace these checks.

## What the normal test set covers

On Linux, the non-ignored `yo-cli` tests create real PTYs and exercise Inline
exit, Fullscreen exit, signal-driven restoration, and two consecutive
`Ctrl+Z`/`SIGCONT` generations in both modes. They do not require tmux, `sshd`,
or an installed Codex:

```bash
cargo test -p yo-cli pty_tests::
```

The process-coordinator tests separately exercise handler installation,
rollback, shutdown compensation, thread ownership, and isolated subprocess
signal behavior:

```bash
cargo test -p yo-cli execution::process::termination::tests
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

## Local tmux and Linux SSH checks

Local tmux on Linux or macOS, both presentation modes:

```bash
cargo test -p yo-cli --test terminal_matrix local_tmux_ \
  -- --ignored --nocapture --test-threads=1
```

Linux SSH and tmux inside SSH, both presentation modes:

```bash
cargo test -p yo-cli --test terminal_matrix ssh:: \
  -- --ignored --nocapture --test-threads=1
```

The local tmux tests are available on both supported Unix hosts and require
compatible installed `tmux` and Codex. The SSH tests remain Linux-only: they
start an isolated localhost `sshd`, generate temporary keys, and remove their
fixture directory. They require compatible local `ssh`, `sshd`, `ssh-keygen`,
Codex, a set `USER` naming the local SSH account, and, for the nested cases,
tmux.

Each route checks both the empty-`Ctrl+D` exit path and two consecutive
`Ctrl+Z` → stopped job → `fg` generations. The job-control checks compare the
terminal with the route's actual interactive-shell termios at every stopped
interval, require the `yo` process to be in the kernel stopped state, and
require the requested presentation mode to be reacquired after each `fg`.
Nested tmux additionally verifies restoration of the outer SSH PTY.

These tests fail when a required command or assertion is unavailable; they do
not convert a missing environment into a successful skip.

## macOS real-host evidence

On 2026-07-30, the tree accepted as `develop` commit `085e763` was exercised
on macOS 26.2 arm64. `cargo test --workspace --all-targets` passed on that
host.

An 80x24 real zsh PTY then exercised both presentation modes. Each mode entered
raw/no-echo input, exited successfully from empty `Ctrl+D`, and completed two
`Ctrl+Z` → stopped job → `fg` generations. Fullscreen left and reacquired the
alternate screen for each generation; Inline never entered it.

The same scenarios passed in tmux 3.6a using `-f /dev/null` and an isolated
socket. Every stopped interval restored the shell termios, and each `fg`
reacquired the requested mode. These were explicit real-host observations, not
part of the normal cross-platform test set.

The SSH routes were then exercised from an 80x24 zsh PTY against the exact tree
accepted as `develop` commit `af546a5`. An SSH-owned interactive zsh ran both
modes through empty-`Ctrl+D` exit and two `Ctrl+Z` → stopped job → `fg`
generations. Inline remained outside the alternate screen; Fullscreen left and
reacquired it for every generation. The local PTY termios was unchanged after
the SSH session exited.

The same SSH session shape also attached to tmux 3.6a with `-f /dev/null` and
an isolated socket. At every stopped interval and final exit, the pane had
returned to zsh, left the alternate screen, and matched its baseline termios.
Each `fg` returned the pane to `yo`, reacquired raw terminal settings, and
restored the requested presentation mode. Exiting the nested session also
restored the outer local PTY. These SSH observations used a real remote host;
they are evidence records rather than part of the normal test set.

## Platform coverage

The executable environment matrix currently covers:

| Host and route | Inline | Fullscreen | Evidence |
|---|---:|---:|---|
| Linux direct real PTY | Yes | Yes | Normal `yo-cli` tests cover exit and repeated suspend/resume in both modes, plus Fullscreen termination |
| Linux local tmux | Yes | Yes | Ignored tests cover clean exit and two shell-driven suspend/resume generations |
| Linux SSH | Yes | Yes | Ignored tests cover clean exit and two remote-shell suspend/resume generations |
| Linux tmux inside SSH | Yes | Yes | Ignored tests cover clean exit, two nested suspend/resume generations, and outer PTY restoration |
| macOS compile | — | — | Workspace all-target tests passed on a real macOS 26.2 arm64 host |
| macOS direct real PTY | Yes | Yes | Real-host run covered clean exit and two shell-driven suspend/resume generations |
| macOS local tmux | Yes | Yes | Ignored tests cover clean exit and two shell-driven suspend/resume generations; a real-host run also covered mode reacquisition and shell termios restoration |
| macOS SSH | Yes | Yes | Real-host run covered clean exit, two remote-shell suspend/resume generations, mode reacquisition, and outer PTY restoration |
| macOS tmux inside SSH | Yes | Yes | Real-host run covered clean exit, two nested suspend/resume generations, pane mode and termios transitions, and outer PTY restoration |

`tools/validation/yo-cli-unix-matrix.sh` checks all `yo-cli` targets on the
current Unix host. Its output describes only that invocation: the current host
is verified and the other host is `unverified(not run on current host)`. This
does not mean that another host is unavailable or erase separately recorded
real-host evidence. The CI workflow runs the equivalent compile check
independently on Linux and macOS; compilation does not replace terminal
behavior evidence.

## Reporting a matrix run

Keep the result small but explicit:

```text
Host:
Route and mode:
Command:
Result: passed | failed | unverified
Observed failure or missing prerequisite:
```

Do not infer one route from another. A passing local tmux run does not mark SSH
or nested tmux on the same host as verified.

Contract: [Rendering validation authority](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.validation-matrix.md)

Return to [Validation](./#reading-a-result) to classify the run as
passed, failed, or unverified.
