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

Use the [explicit free image definition](../workflows/provider-catalogs/openrouter.md#explicit-free-image-connection)
and isolated config, credentials and Session state. Verify private-socket Ctrl+V,
attachment preview, submission, streamed completion, correlated local tool
results, idle `/compact`, normal exit and a fresh `--continue` response. Inspect
the exact PNG occurrences, NVIDIA-only route, zero-price caps and disabled
fallback on every request. Unit tests and a local fixture establish projection
and recovery; actual provider execution requires a live run.

On 2026-09-13, the stock Fullscreen TUI and a real Linux PTY completed the
following checks against `nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free`:

| Check | Result |
|---|---|
| Import and private-socket synthetic 64 × 32 PNG | Passed |
| Red-left/blue-right description and streamed completed Turn | Passed |
| One real `read_file` call and independently checked marker/result | Passed |
| Second Turn and image-aware idle summary checkpoint | Passed |
| Fresh continuation after summary and retained image colors | Passed |
| Exact initial marker returned after the live summary | Failed |
| Final successful TUI exit and termios restoration | Passed |

The connector retains automatic function-tool selection without explicit
`tool_choice: auto` on this image profile. A final one-choice accounting frame
is accepted only after semantic finish, with the same response id, index 0,
repeated finish reason and an empty closed role/content delta. It records usage
once without repeating output or changing the earlier terminal status.
`[DONE]`, sum validation, limits and rejection of duplicate/trailing data remain
mandatory. Connector tests (34) and managed-backend tests (84) passed.

The live binary SHA256 was
`9e9a7e2443ec20009ca70cd339ba47974beef85d393b26207a2550331d952575`. Sixteen
actual requests were recorded: twelve diagnostics before the selector correction
and four requests validating the corrected implementation. Two earlier responses
were cancelled by the harness and are excluded from completion evidence; their
final usage was not observed. The initial harness replayed historical snapshot
records and also blocked on undrained PTY output. The CLI now recovers matching
checkpoint-only discovery candidates read-only before selecting them;
unsupported, unavailable and unrelated execution identities are not promoted.
The final scenario deduplicated Journal sequences and checked the exact new Turn
id and response facts. The image colors survived; the final response did not
contain the exact initial marker. Full summary fidelity is therefore not
established, and no defect in replay serialization was established by this run.
Every observed completed usage report had cost 0; all requests retained the
closed free route. No provider retry or fallback ran. The intermediary forwarded
SSE unchanged, recording only structural facts and usage. Ordinary Yo
configuration was unchanged, and temporary credentials, Session state, synthetic
files, sockets, intermediary and TLS keys were removed.

A bounded follow-up on candidate `d2e48da0` used one of at most four new live
requests and stopped on a failed first Turn. HTTP status 200 was observed, but
no assistant content or final usage was captured. Stream EOF alone is not a
successful semantic terminal. The run never reached summary or fresh
continuation, so it neither locates the earlier marker loss nor establishes
zero reported cost for that request. No automatic retry or fallback ran, and
the proxy and isolated state were removed.

On the same candidate, an offline real-PTY oracle completed three distinct
Turns, one image-aware checkpoint and fresh-process continuation. It compared
the literal marker in summary input, checkpoint storage and resumed input;
the exact synthetic summary was present in the resumed request. Both exits
returned status 0 and restored termios. This verifies the transport and recovery
seams, not provider summary fidelity. The binary SHA256 was
`266e7ba7a6d85271c7d42a91a68dbcac47e7ef5fee15fabac725fdaa0fdefaab`, and all
26 focused managed context/replay tests passed.
An HTTP-200 response carrying a synthetic SSE error code 502 was also rejected
as a failed Turn. The negative control restored termios and removed its state;
it did not count the HTTP status or `[DONE]` alone as a successful response.

The configured Mac also passed profile compile/import, tmux attachment and SSH
PTY lifecycle checks. Physical keys, IME and actual Command-V passed the
[direct input verification](#current-mac-direct-input-verification). These Mac
checks establish terminal behavior; the live provider checks above ran on Linux.

## Diagnose a managed request failure

Run the deterministic pre-acceptance failure check without a provider account:

```bash
cargo build --locked -p yo-cli
python3 tools/validation/managed-start-failure.py target/debug/yo --mode inline
python3 tools/validation/managed-start-failure.py target/debug/yo --mode fullscreen
```

The [runner](https://github.com/Yon-Fandorin/yo/blob/develop/tools/validation/managed-start-failure.py)
imports a text-only Chat Completions binding with a fake key and a loopback-only
endpoint. Its local socket rejects TLS before HTTP; it never forwards traffic
or changes certificate trust. Each mode submits once and checks exit status 1,
typed `Transport` stderr, saved `last_failure.kind: transport`, zero accepted
requests and zero finished Turns, and restored PTY modes. Fullscreen also checks
the alternate-screen enter/leave pair. A shell keeps the controlling-terminal
session alive until the parent has captured restored modes; a private pipe
acknowledges that capture. On Darwin, the comparison excludes only `PENDIN`,
the kernel's pending-input state bit set when returning to canonical input,
as shown in [Apple's tty implementation](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/tty.c).
Every other termios field is compared. The JSON diagnostic is collected before
isolated configuration, credentials, host identity and Sessions are deleted;
failed checks also report cleanup. Ordinary user state is not used.

On 2026-09-12, both Linux modes passed: one loopback connection attempt, exit
status 1, the typed stderr and stored failure, zero accepted requests or finished
Turns, and terminal restoration. A deliberately failed import also returned a
failure diagnostic and removed its temporary state. All 13 shared transport
tests passed, including the status-only HTTP failure classification.

On the same date, candidate `0f86d85e` built and passed both Inline and Fullscreen
on macOS 26.6.2 arm64. Each mode retained one submission, one loopback connection
attempt, Yo exit status 1, typed `Transport` stderr, and the stored `transport`
failure. There were zero HTTP requests, accepted requests or finished Turns.
Both modes restored all PTY settings except the kernel's `PENDIN` state bit;
Fullscreen emitted exactly one alternate-screen enter/leave pair, and Inline
emitted none. Each isolated state root and the temporary checkout were removed.
Ordinary config/credential hashes remained unchanged. This closes Mac coverage
of the shared pre-acceptance failure diagnostic; live OpenRouter inference and
the earlier live exit's specific cause remain unverified.

`BackendRequestAccepted` counts requests whose connector start succeeded. The
[transport worker](https://github.com/Yon-Fandorin/yo/blob/develop/crates/connectors/transport/src/worker.rs)
can send HTTP and then return a status or transport error before that record is
committed. The CLI propagates that failure and restores the terminal before
exiting. Keep submission, connection/HTTP attempt, acceptance and finished-Turn
counts separate. Before cleaning a live harness, collect the child exit status,
secret-safe stderr and the binding's typed failure; retain the tmux pane until
those observations are captured. Zero accepted records alone cannot establish
zero HTTP attempts.

This fixture verifies the shared failure and diagnostic path, not an OpenRouter
image binding or a provider HTTP rejection. The earlier Mac live run's specific
cause cannot be recovered from its deleted captures and remains unverified.

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
tests. The suite checks exact `0.153.4` and `0.154.0` wire evidence, exact selected-model
`inputModalities`, ordered repeated immutable PNG projection for both start and
steer, conservative inherited-history resume/rebind admission, and the complete
32 MiB outbound JSONL boundary. The boundary test accepts exactly 32 MiB and
rejects the first excess byte before the peer sees any bytes; server responses use
the same boundary and classify overflow as protocol failure. These tests use fake
JSONL peers and do not contact a model service.

On 2026-09-10, an isolated official Codex run with `gpt-6-astra` also passed
two synthetic model requests: an image turn and fresh-process resume. The
clipboard fixture produced the exact `red,blue` answer; the resumed turn passed
the exact recall check, retained two continuation anchors and preserved the
existing journal prefix. The tested binary SHA256 was
`a20df491ff6445f521bb45d8f03f67f79fb7f72dbea355312ac36c9af394603d`.
The owned tmux session was closed and temporary authentication was removed.
This establishes the tested image-input and recovery route; it does not verify
outer-terminal image pixels or other model/version combinations. Both requests
in the approved two-request budget were consumed.

On 2026-09-11, installed Codex `0.154.0` passed image-wire compatibility checks
with external networking disabled in a loopback-only namespace. Schema bundles
generated by the installed `0.153.4` and `0.154.0` executables had identical
`TurnStartParams`, `TurnSteerParams`, and `ModelListResponse` definitions. The
exact reviewed-version list now includes both versions; adjacent patches and
prerelease versions remain unknown for image input.

The native app-server accepted a 1,080,993-byte synthetic PNG in both `turn/start`
and `turn/steer`, preserving image bytes and surrounding text order through two
local mock Responses requests. A separate real Rust Yo run in isolated 96×32
tmux acquired an image through Ctrl+V and completed a turn, then exited and
resumed in a fresh process. The mock endpoint received the exact immutable PNG
from the Yo journal on both turns (a 1,921,054-byte data URI); two continuation
anchors were present and the journal prefix was preserved. The tested Yo binary
SHA256 was `a4b4173d6ddba7ef9a092e960d229a93eaf028b315b43809749570d83a457aa5`.

These checks used the installed host and a synthetic local model endpoint,
without credentials or external model requests. They establish transport and
continuation compatibility, not live-service visual recognition. The adapter
suite also verifies selected-model modality admission and rejection before native
resume when the retained-image target is text-only. Existing message-size and
immutable-input checks remain in force.

### Saved command rules and automatic approval

On 2026-09-11, Linux Rust yo in isolated 110×44 tmux passed four synthetic
turns against official Codex `0.154.0` with `gpt-6-astra`. The binary SHA256 was
`f72d343be2d2aecef3f844033a6e84fa59aa3ffc902c680ced0a6b5bc6200d6a`.
The delegated Codex package tests and CLI build passed before inference.

With `approval_policy = "on-request"`, `approvals_reviewer = "user"` and
`sandbox_mode = "read-only"`, the first turn selected the actual TUI's
`Approve + save rule` choice for one disposable script's exact absolute path.
The native rules file contained only that allow prefix, and the script appended
exactly one marker. Resizing the pending panel to 40, 20 and 110 columns neither
executed the script nor saved a rule. After graceful exit and fresh-process
resume, the same command appended one more marker without another approval.
The physical Yo journal prefix and saved rule were unchanged.

A different script produced a new approval request. Selecting `Decline and stop`
left its output absent, kept the saved rule unchanged and interrupted the turn.
The original driver then attempted to resume this cancelled suffix; the current
continuation guard correctly opened read-only history because no newest durable
anchor existed. This driver sequencing failure was retained. Its three completed
scenarios were not resent; the fourth scenario used a fresh Session.

For that Session, `approvals_reviewer = "auto_review"` used the same read-only
sandbox with an empty rules directory. A single synthetic append completed
without a manual approval. Captured `item/autoApprovalReview/completed` evidence
reported `decisionSource: "agent"` and `approved`, correlated to the exact
command item, thread and turn before command completion. The file contained
exactly one marker, and no rule was saved. This checks the host's
[automatic review route](https://learn.chatgpt.com/docs/sandboxing/auto-review),
not a grant inferred from the absence of an approval panel.

An offline audit matched offered ordinals, exact wire decisions, native command
results and Yo request/response identities. It rejected ten altered-evidence
controls covering choice, rule scope, cleanup, request identity, command status,
review status, target, ordering, decision source and missing review. Temporary
authentication, native state and rules were removed; workspaces were emptied,
owned tmux servers closed and no owned processes remained. The four `turn/start`
calls do not count internal model or reviewer requests.

### Automatic refusal and network approval scopes

On the same date, the same Yo binary and official Codex `0.154.0` passed ten
additional turns through an isolated 120×48 tmux TUI. A credential-free local
Responses fixture supplied deterministic tool calls and reviewer responses;
there were 20 local model/reviewer HTTP requests and zero real-service requests.
This verifies native policy execution, adapter behavior and visible TUI results;
it does not measure the service model's risk classification.

Actual-service automatic-approval risk classification remains unverified; no
free Codex service route was established for this check. This is separate from
choosing an independent code-review model for a task. The existing fixture
results do not claim the real reviewer will make the same decisions.

Network cases used a named permission profile with the native proxy enabled and
an isolated `[experimental_network]` requirements fixture. A mount namespace
exposed the temporary requirements only to test processes; the real `/etc/codex`
and the user's Codex configuration were unchanged. The target was a loopback
HTTP server that counted received requests.

- Selecting the offered session-only grant allowed one request. Another turn in
  that running Session reached the same target without an approval; a fresh
  process and Session required approval again. Cancelling reached no target.
- Selecting the offered persistent network allow saved exactly one native
  `network_rule` for the loopback host and HTTP protocol. A fresh process and
  Session reused it without approval. A different hostname still required
  approval, and cancelling reached no target.
- An explicitly seeded network deny rule blocked both the initial and
  fresh-process requests without approval or a target hit. Yo displayed the
  native failure explaining that policy explicitly denied the domain. The rule
  remained unchanged.
- An injected reviewer denial produced a native `denied` review and a `declined`
  command. A stalled reviewer response reached the native 90-second deadline,
  producing `timedOut` and a failed command. Neither created the command's marker
  file or requested manual approval; both displayed `Codex approval warning` in
  Yo, with the corresponding denial or timeout explanation.

Codex `0.154.0` proposed both network amendments in metadata but exposed only
allow in `availableDecisions`. Therefore saving a persistent deny through the
actual TUI remains unavailable in this host version. The seeded-rule check
establishes enforcement, not an interactive deny-save journey. The adapter test
`offered_approval_choices_preserve_exact_scopes_and_wire_payloads` separately
covers exact allow/deny transmission when the host offers those choices.

An offline audit checked the ten outcomes, offered ordinals and exact responses,
process boundaries, persisted rules, target receipts, visible warnings and
review-to-command identities and ordering. Eight altered-evidence controls were
rejected. Temporary native state, test rules and workspaces were removed, owned
processes and tmux servers closed, and loopback listeners released. No runtime
code changed; the prior package/build results still apply.

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
No prompt was submitted or clipboard read. This initial check establishes
authentication and empty Session startup only. The subsequent skill and resume
check is recorded below; the native read-only sandbox remains unavailable.
The normal Grok suite separately verifies that `session/load` drains 1,025
matching historical updates before its response, then delivers a fresh response
and resumable outcome. Other sessions, server requests, correlation failures,
and the original unrelated-message backlog bound remain enforced. This is a
deterministic adapter test; the measured live context-resume check is below.

An initial isolated synthetic skill prompt exposed an unsolicited, methodless
`skills-reload` maintenance response from Grok's native skill watcher. Yo rejected
its string ID before recording a continuation anchor. Commit `f94d1a42` consumes
the exact maintenance acknowledgement shape while preserving numeric request
correlation. Adapter tests cover startup, active prompts and resume, including
rejection of unrelated responses and no early prompt completion.

The corrected binary passed an isolated official Grok run on 2026-09-10 with
two synthetic model submissions and two observed watcher acknowledgements.
The first skill response matched exactly. After removing the skill source,
fresh-process resume also passed exact recall, retained two continuation anchors
and preserved the journal prefix. The tested binary SHA256 was
`f72d343be2d2aecef3f844033a6e84fa59aa3ffc902c680ced0a6b5bc6200d6a`.
The owned tmux session was closed and temporary authentication was removed.
The installed host advertises no image prompt support; a native read-only review
sandbox remains unavailable.

### Grok large-context resume

On 2026-09-11, the same binary and Grok `1.0.25 (f7e67d6988e2)` passed seven
synthetic submissions against the official service in isolated Linux 110×40
tmux. The delegated Grok suite passed 64 tests with two environment tests ignored;
the previously passing CLI build was unchanged. Tools, web search, subagents and
memory were disabled; authentication and host state used a disposable Grok home.

Six turns supplied 960 synthetic records totaling 120,272 input bytes. Each
returned its exact acknowledgement. After graceful exit, a fresh Yo process
resumed the same Yo Session and native Grok Session, then correctly returned
three random checkpoint values from the early, middle and final batches. The
resume prompt named the checkpoints without supplying their values. All seven
turns completed, seven continuation anchors were retained, and the 192,069-byte
physical journal prefix remained byte-identical. No tool or approval ran.

Native `session/load` replayed 19 updates before its correlated response; that
response preceded the seventh `session/prompt`. Replayed messages did not create
duplicate Yo answers. An offline audit checked exact answers, input size, Session
and process identity, journal prefix, anchors, ordering and cleanup, and rejected
six altered-evidence controls. The seven planned submissions were all consumed
without retry. Temporary authentication, native state and owned processes were
removed after verification.

This establishes resume at the measured input size and seven-turn depth. The
1,025-update mailbox boundary remains a separate deterministic test; this run
does not establish a live replay of that many updates, context compaction or
behavior at the host's maximum context window.

## Local tmux and Linux SSH checks

An earlier user observation confirmed that the mountain image was visible and
an HTTP link opened in their SSH/tmux terminal. The remote README file link did
not open. That file-link limitation is documented in the repository README;
there is no remote file transfer or built-in viewer. This is a user-reported
observation with no retained terminal/version matrix, not an automated pixel
check or a claim about every terminal. The earlier local "awaiting pixel
confirmation" notes are superseded by that response.

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

Local tmux launches `yo --model host:codex` under an interactive Bash with a
temporary `HOME`, `CODEX_HOME`, XDG roots, Yo configuration and Session repository.
The Codex provider points only to a reserved loopback listener, uses no account
and keeps authentication storage file-only. The tests wait for the actual input
screen and raw/no-echo mode rather than tmux's foreground-command name. Empty
`Ctrl+D` must restore the shell terminal and main screen before the shell exits
with status 0. On macOS only, the transient queued-input `PENDIN` bit is excluded
from the otherwise complete termios comparison. No backend thread binding,
accepted request, finished Turn or loopback connection may occur. Each test
removes its own tmux server, socket and temporary Codex/Yo state.

To mock application-side typing and terminal text paste without model inference:

```bash
cargo test -p yo-cli --test terminal_matrix draft_input_and_bracketed_paste \
  -- --ignored --nocapture --test-threads=1
```

Both modes receive ASCII typing and Backspace, completed Korean characters with
cursor movement and Delete/Backspace, and an LF/CRLF paste containing Korean and
an emoji. The paste must display three distinct draft rows without submitting a
Turn. `Ctrl+C` clears each draft; empty `Ctrl+D` verifies exit, terminal restoration
and cleanup. The fixture uses a private tmux buffer and
[`paste-buffer -p -r`](https://man.openbsd.org/tmux#paste-buffer) to request paste
bracketing and preserve linefeeds. It does not use the system clipboard.
This tests decoded characters and terminal paste; physical key mapping,
macOS IME preedit/commit and terminal Command-V were covered separately by the
[direct input verification](#current-mac-direct-input-verification).

Each route checks both the empty-`Ctrl+D` exit path and two consecutive
`Ctrl+Z` → stopped job → `fg` generations. The job-control checks compare the
terminal with the route's actual interactive-shell termios at every stopped
interval, require the `yo` process to be in the kernel stopped state, and
require the requested presentation mode to be reacquired after each `fg`.
Nested tmux additionally verifies restoration of the outer SSH PTY.

These tests fail when a required command or assertion is unavailable; they do
not convert a missing environment into a successful skip.

### Current Codex Mac tmux verification

On 2026-09-12, the unchanged `6aac838b` candidate tree
(`a7dfab60f1caea706c0fc9dbe02f50ba90d4fc64`) passed all six local tmux tests on
macOS 26.6.2 arm64 with installed Codex 0.154.0 and tmux 3.6a:

```bash
cargo test --locked -p yo-cli --test terminal_matrix local_tmux_ \
  -- --ignored --nocapture --test-threads=1
cargo clippy --locked -p yo-cli --test terminal_matrix -- -D warnings
```

Inline and Fullscreen each entered raw/no-echo input, exited status 0 on empty
`Ctrl+D`, restored shell termios and the main screen, and completed two
stopped-job/`fg` generations. The two input mocks also passed ASCII and
completed-Korean editing, three distinct LF/CRLF paste rows containing Korean
and an emoji, and `Ctrl+C` draft clearing. Native Clippy passed. Thread bindings,
accepted requests, finished Turns and loopback inference connections remained zero.
Ordinary Yo state and the read-only Mac source repository were unchanged. All
test-owned tmux resources and temporary state, followed by the disposable
checkout, packet and build/log files, were removed. Physical keys, IME
preedit/commit and terminal Command-V were checked separately below.

## Current Mac direct input verification

On 2026-09-13, candidate `fb396aa0` built with the locked dependencies on the
configured arm64 Mac. In the user's existing tmux pane `%17` (`mac_yo`), the
user confirmed normal Korean/English switching, IME composition and Backspace,
direction-key editing, and two-line text pasted through the terminal's actual
Command-V. Ctrl+C and then empty-prompt Ctrl+D each exited with status 0;
recorded termios values were restored, allowing macOS's queued-input `PENDIN`
flag. The tested binary SHA256 was
`f07383917d56c280750e7524207832cb48f32ab1e022a85ffa1676b086d509ad`.

The stock Fullscreen TUI used a fake credential and isolated config/Session
state under a native macOS sandbox denying network access and writes outside
the owned temporary directory. This establishes physical terminal input and
exit behavior. It does not establish model-service behavior. The temporary
checkout, build, input state and failed old-binary probe were removed; the
user's tmux pane, installed Yo and clipboard setup were preserved.

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

### Current Apple Silicon build and input check

On 2026-09-11, the corrected tree based on `f57e61e5` was checked on macOS
26.6.2 arm64 with the pinned `nightly-2026-05-22` toolchain.

The native core suite passed 706 tests. CLI checks passed 524 unit tests and
seven integration tests, with 18 environment tests ignored. Clippy passed for
`yo-core`, `yo-backend-delegated-grok` and `yo-cli`; the host-target Unix matrix
and native offline `chat_preview` build also passed. Linux validation passed
706 core tests, 64 Grok tests, 532 CLI unit tests and seven CLI integration tests;
the updated workspace-reference fixtures passed 31 focused tests. These totals
overlap and must not be added together as independent coverage.

The user opened the offline executable in a Mac tmux pane and explicitly
requested automated input there. Injected input verified Korean text, cursor
movement, Backspace and insertion, bracketed multiline paste, clearing the
draft, empty `Ctrl+D` exit, and a subsequent shell command with canonical input
and echo restored. The user-owned tmux pane remained at its shell. The native
preview binary SHA256 was
`34ebe6cf4633d15e36d826ff8ce9774ec63aa2a7014027a35fa753cd6eb04a85`.
No model request or account access was involved. This is an actual Mac tmux
route with injected input; physical keys, IME composition and the terminal
application's Command-V handling were verified separately in the
[direct input verification](#current-mac-direct-input-verification).

Disposable checkouts, source-transfer artifacts and test processes were cleaned
up. The user's tmux session and existing clipboard installation were preserved.

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
