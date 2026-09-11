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

## OpenRouter image journey

For the explicit free image profile, verify definition import, Ctrl+V from a
private clipboard socket, visible attachment preview, Enter submission, streamed
completion, clean exit, and `--continue` in a fresh TUI process. Compare the
normalized PNG data URLs and free routing fields on the first and resumed
requests. Keep configuration, credentials and Session storage isolated.

The Linux PTY smoke on 2026-09-11 passed this journey with two requests to a
local TLS/SSE fixture. The test process redirected the exact OpenRouter hostname
to loopback, denied other TCP destinations, used a temporary test CA and a fake
key, and removed its state after exit. This verifies Yo's transport and recovery;
live NVIDIA streaming, tool execution, summary completion and the new profile
on macOS/SSH/tmux remain separate environment checks. Connector unit tests cover
tool-result and bounded summary projection; they do not prove provider execution.

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

The delegated Codex image boundary is verified offline in the adapter and core
tests. The suite checks exact `0.153.4` wire evidence, exact selected-model
`inputModalities`, ordered repeated immutable PNG projection for both start and
steer, conservative inherited-history resume/rebind admission, and the complete
32 MiB outbound JSONL boundary. The boundary test accepts exactly 32 MiB and
rejects the first excess byte before the peer sees any bytes; server responses use
the same boundary and classify overflow as protocol failure. These tests use fake
JSONL peers and do not contact a model service.

## Installed Grok checks

Verify ACP initialization, cached-login authentication, and cleanup without creating
a Session or requesting model inference:

```bash
cargo test --locked -p yo-backend-delegated-grok \
  runtime::tests::session::local_grok_authenticates_and_shuts_down_without_a_session \
  -- --ignored --exact
```

This check passed on Linux with Grok `1.0.25 (f7e67d6988e2)` on 2026-09-10.
An isolated 96×32 tmux run also reached the empty composer through the actual
Rust `yo --fullscreen --model host:grok`, then exited cleanly with Ctrl+D.
The tested binary SHA256 was
`42f7c955d966d56825213c18a8ce59c7d655acb17532fa606dc2f449f2b83d7a`.
No prompt was submitted or clipboard read. This establishes authentication and
empty Session startup only; authenticated turns, actual retained
history, skills, and the native read-only sandbox still need their own evidence.
The normal Grok suite separately verifies that `session/load` drains 1,025
matching historical updates before its response, then delivers a fresh response
and resumable outcome. Other sessions, server requests, correlation failures,
and the original unrelated-message backlog bound remain enforced. This is a
deterministic adapter test, not an actual long Grok session measurement.

A subsequent isolated synthetic skill prompt completed with the expected marker
in Grok's native record, but Yo failed with an unsigned response-ID protocol error
before recording a continuation anchor. Fresh-process resume was not attempted;
the accepted request was not resent. Offline leader-mode Responses API tests with
explicit skill text returned numeric IDs and did not reproduce the failure.
The adapter now reports the invalid ID's JSON type without exposing its value;
this improves diagnosis but does not fix or establish the cause of that failure.

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

## Mac clipboard to Linux yo

On 2026-09-10, candidate `8343292f0281b9e8d7321fd504bed97ead205b7c` passed
an image attachment check from macOS 26.6.2 arm64 through SSH Unix-socket forwarding
to the actual Linux Rust binary in isolated tmux. The Mac used `pngpaste` 0.2.3 and
the candidate's `tools/clipboard_bridge.py`; explicit Homebrew paths were required.
The helper's installation hash was
`5e3fa05d25b1d856a89927dc1e5cd236d9801b80a4c6ac42af6704ace0b57c58`.

A synthetic 64 × 32 PNG was placed in the Mac general pasteboard while its previous
materialized items stayed in Mac memory. A temporary guard buffered real `pngpaste`
output on the Mac and checked the fixture pixels and unchanged pasteboard revision
before exporting any bytes. Ctrl+V delivered through `tmux send-keys` attached the
image at 20, 40 and 96 columns. After restoring the previous pasteboard, the guard
rejected another capture and yo preserved both the existing image and the draft.
No model Turn was submitted. The application exited successfully and the owned
SSH connection, helper, sockets, tmux and fixture were cleaned up.

The connection launcher was also checked separately in a Linux real PTY for two
Ctrl+Z → foreground cycles and empty-Ctrl+D exit, with shell terminal settings
restored. These observations establish native acquisition, transport and Rust input
routing; they do not verify a physical Mac keyboard shortcut or a terminal's pixel
image protocol. A clipboard source still requires explicit per-process selection.

The native configured SSH path subsequently passed the same Mac fixture, attachment
widths, failed-paste preservation and clean-exit checks without a Python launcher,
bridge service or forwarded socket. That Rust binary had SHA256
`f124a62a87ab04c06a163fa9c6752859eb4171cd0debaeaf5fbde07f3b8739cc`.
A test-local SSH shim selected the guarded native reader; the production worker,
SSH authentication and embedded remote supervisor performed the acquisition.
The previous clipboard was restored, the temporary fixture was removed and no
model request was sent. The Python supervisor's separate synthetic suite covers
reader/descendant cleanup on timeout and disconnect signals, and rejects partial
output from failed captures.

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
