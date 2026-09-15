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

## Nonsecret interview recovery on the saved Mac

On 2026-09-15, the accepted nonsecret interview implementation and its two
test-only portability corrections were checked on the pinned macOS arm64 host.
The first run exposed `/var -> /private/var` in test fixture paths and a
nanosecond-name collision between parallel model tests. Commits `d640857f` and
`640c7c1a` make those fixtures use physical temporary roots and UUID identities;
the repository's descriptor-based symlink rejection remains unchanged.

The default profile passed 724 Core tests, Core all-target Clippy, 529 CLI unit
tests plus five and two integration tests, and the macOS Unix compile matrix.
The final `640c7c1a` tree passed six frame tests, two zero-size reentry tests,
940 TUI unit tests, four rendering tests, TUI all-target Clippy, 78 filesystem
tool tests, and CLI all-target Clippy. These totals overlap and are not additive.
The interview tests cover recover, reopen, local edits, explicit send, durable
acceptance, conflicts and recovery failures on the Mac filesystem.

An isolated local-tmux run also passed Inline and Fullscreen empty-`Ctrl+D`
exit, two suspend/resume generations in both modes, completed-Korean editing,
cursor/delete operations, multiline bracketed paste and `Ctrl+C` clearing.
The first cold Fullscreen input attempt exceeded its five-second readiness
bound while `yo` was still starting; the other five scenarios passed, and the
exact input scenario passed after one successful Fullscreen warm-up. No model
request occurred. Physical IME and Command-V were repeated on the accepted candidate
in the [current direct input verification](#current-mac-direct-input-verification).
A live-provider send remains unverified.

## Large-body paging on the saved Mac

On 2026-09-13, clean candidate `af16336de8836475af39022d08bf612d13955110`
passed the saved Apple Silicon Mac's `yo-tui` validation profile. The exact
Git bundle was checked and built in a disposable checkout with the pinned Rust
toolchain. Frame tests (6), zero-size reentry tests (2), all-target TUI tests
(921 unit and 4 integration tests) and all-target Clippy with `-D warnings`
passed. The suite covers 70,000-row individual bodies, large table cells and
expanded diffs, narrow-width navigation and complete paged Inline publication
failure recovery. These automated Mac checks do not repeat physical IME or
clipboard-key verification. The installed app and normal credentials were
unchanged; the owned checkout, bundle and runner were removed.

## OpenRouter image journey

Use the [explicit free image definition](../workflows/provider-catalogs/openrouter.md#explicit-free-image-connection)
and isolated config, credentials and Session state. Verify private-socket Ctrl+V,
attachment preview, submission, streamed completion, correlated local tool
results, idle `/compact`, normal exit and a fresh `--continue` response. Inspect
the exact PNG occurrences, NVIDIA-only route, zero-price caps and disabled
fallback on every request. Unit tests and a local fixture establish projection
and recovery; actual provider execution requires a live run.

On 2026-09-13, the stock Fullscreen TUI and a real Linux PTY completed the
following checks against `nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free`.
The table includes the final bounded fidelity run described below:

| Check | Result |
|---|---|
| Import and private-socket synthetic 64 × 32 PNG | Passed |
| Red-left/blue-right description and streamed completed Turn | Passed |
| One real `read_file` call and independently checked marker/result | Passed |
| Second Turn and image-aware idle summary checkpoint | Passed |
| Fresh continuation after summary and retained image colors | Passed |
| Exact initial marker returned after the live summary | Passed in final bounded run |
| Final successful TUI exit and termios restoration | Passed |

The connector retains automatic function-tool selection without explicit
`tool_choice: auto` on this image profile. A final one-choice accounting frame
is accepted only after semantic finish, with the same response id, index 0,
repeated finish reason and an empty closed role/content delta. It records usage
once without repeating output or changing the earlier terminal status.
`[DONE]`, sum validation, limits and rejection of duplicate/trailing data remain
mandatory. Connector tests (34) and managed-backend tests (84) passed.

Earlier diagnostics observed a missing marker, unobserved final usage for
interrupted responses, and an embedded SSE error 502 despite HTTP 200. They did
not establish a replay-serialization defect. The final run below establishes
summary fidelity for its bounded scenario. The CLI validates matching
checkpoint-only discovery candidates by read-only recovery; unsupported,
unavailable and unrelated execution identities are not promoted. Offline real-PTY
success and embedded-502 controls check exact checkpoint/final-Journal equality
and termios restoration without inference. HTTP status or `[DONE]` alone never
establishes successful semantic completion.

The configured Mac also passed profile compile/import, tmux attachment and SSH
PTY lifecycle checks. Physical keys, IME and actual Command-V passed the
[direct input verification](#current-mac-direct-input-verification). These Mac
checks establish terminal behavior; the live provider checks above ran on Linux.

### Completed image-summary fidelity

The user-requested run on clean candidate `0197d6f4` completed all four of its
four permitted requests against the same NVIDIA-only zero-price image route.
The stock binary SHA256 was
`22cb8cb9587a5fd1eebbb31e67372f8d3dc3cb73324c5b4afdbb95daa7e898a0`.
Current official model and endpoint inventory confirmed image input, function
tools and the sole NVIDIA route with prompt/completion price 0 before dispatch.
All 34 Chat Connector and 85 managed-backend tests passed. On this same binary,
offline success and embedded-SSE-502 controls passed without external inference.

| Request | Input tokens | Output tokens | Total tokens | Reported cost |
|---|---:|---:|---:|---:|
| Image input and initial literal reference | 1278 | 143 | 1421 | 0 |
| Second Turn | 1337 | 24 | 1361 | 0 |
| Image-bearing idle summary | 621 | 1344 | 1965 | 0 |
| Fresh-process continuation | 1277 | 240 | 1517 | 0 |
| Total | 4513 | 1751 | 6264 | 0 |

Every stream had semantic `stop`, mandatory `[DONE]` and final usage, with no
tool call. The initial and summary requests each carried the canonical PNG.
The literal `YO_FIDELITY_LITERAL_6D82A31F` was present in summary input, summary
output, stored checkpoint, resumed input and final completed Journal message.
The checkpoint's portable body equalled the complete summary response exactly;
the resumed request included that exact summary, and the final Journal bytes
equalled the final response. That final answer retained both the literal and
the image color names red and blue. Three distinct Turns and one checkpoint
completed; both TUI processes exited 0 and restored termios.

This closes the outstanding live image-summary fidelity check for this bounded
scenario. It does not erase the earlier marker-loss or 502 observations above,
establish universal summary fidelity, or guarantee future free-route capacity.
There were no automatic retries, fallback, changed routing caps or additional
inference requests. The proxy, synthetic files, isolated credentials, Session
state and TLS keys were removed.

## QwenCloud renewal and image capability

On 2026-09-13, candidate `9eb0bbf9` refreshed the existing authenticated
QwenCloud account and observed its renewed Standard Token Plan. The refresh
updated the normal account-capacity cache; it was not an inference request.
All 12 QwenCloud provider tests and 57 Responses connector tests passed.

One direct Responses API request used the configured `qwen3.8-max` model and
the exact international Token Plan endpoint, without redirects, retries or
pay-as-you-go fallback. A synthetic 64 × 32 red-left/blue-right PNG produced
HTTP 200, `response.completed` with status `completed`, and the exact color
answer. Response `resp_ebd49743-e297-4633-ad04-7b4f0154f692` reported 148 input
and 56 output tokens, 204 total. This used the user-authorized subscription;
no zero-cost or general-API free-quota claim is made.

This passes Provider API image capability only. The separate general API Yo
image journey is verified below. General-API free quota is separate from the
[Token Plan quota](https://www.alibabacloud.com/help/en/model-studio/new-free-quota).
Temporary capability probes were removed after recording this result.

## QwenCloud general image journey

Use the [explicit general API image connection](../workflows/provider-catalogs/qwencloud.md#explicit-general-api-image-connection)
and a real Linux PTY with the stock Fullscreen TUI to verify private-socket Ctrl+V,
preview, submission, function tools, idle `/compact`, normal exit and fresh
`--continue`. Keep the accepted exact Flash envelope without replacing it with
a plan account or another model.

On 2026-09-13, all five of the five permitted requests completed against
`qwencloud:general:qwen3.8-flash` at the international Chat endpoint.
Authenticated read-only quota checks observed `Free quota only` on both before
and after execution. Free capacity decreased from 988,173 to 982,591 tokens,
exactly matching completed usage. No provider setting change, redirect, retry
or fallback ran. Responses had no cost field; no zero reported-cost claim is made.

| Request | Input tokens | Output tokens | Total tokens |
|---|---:|---:|---:|
| PNG and real file tool call | 1024 | 48 | 1072 |
| Correlated tool result and first answer | 1139 | 22 | 1161 |
| Second Turn | 1207 | 22 | 1229 |
| Image-bearing idle summary | 589 | 265 | 854 |
| Fresh-process continuation | 1245 | 21 | 1266 |
| Total | 5204 | 378 | 5582 |

The synthetic 64 × 32 PNG's red-left/blue-right pixels were checked independently.
A real `read_files` call id matched its completed result and the exact workspace
marker contents. `YO_QWEN_IMAGE_LITERAL_81C64E2A` and both colors survived summary
source, response, stored checkpoint, resumed input and the final sealed Journal.
The checkpoint equalled the complete summary bytes; resumed input contained that
exact summary, and the final Journal equalled the final response bytes. The first
four requests each carried the same canonical PNG once; the resumed request had none.

Both thinking options were exact boolean `false` on all five requests. Enabled
tools used explicit `tool_choice: auto`; summary omitted tools and tool_choice.
Every positive output cap was 131072. Each stream completed index 0, semantic
finish, final usage and mandatory `[DONE]`. Three distinct Turns and one
checkpoint completed. Both processes exited 0, left Fullscreen and restored
exact termios. All four accounting fields survived the checkpoint; the
image-free successor retained Qwen advisory policy with reserve 0.

Offline success and HTTP-200 embedded-502 controls passed on the same binary
without inference; the failed Turn never counted as a successful response.
Chat Connector tests (39), managed backend (85), core model service (176),
the full workspace and Clippy passed. The binary SHA256 was
`ef9462786b6fb7dfccc40ad789bfa1a09440d3a880dcf54ffd60602f3ff4db46`.
The intermediary read the actual key only into memory from normal Yo credential
storage; isolated Yo used a synthetic local key. The normal general API key and
distinct Token Plan account were preserved. Disposable config, Sessions, socket,
synthetic files, intermediary and TLS keys were removed after promoting the result.
This is evidence for the bounded scenario, not a guarantee for every future
summary or free-quota availability.

Native Codex 0.154.0 reviewed implementation commit `d0a627cc` with
`gpt-5.6-sol`, effort `high`, and returned `CLEAR` in Session
`01a09ac4-b2b8-7e20-b2a5-c201131c69b4`. The first invocation failed while
decoding the response body and produced no verdict; the user approved exactly
one additional fresh-Session transmission of the same immutable 285,804-byte
input (SHA256 `e366488e9fd3258b262f97ff86a830e566e0779e0ea9ab9fae9b5f53c4a2fcc5`).
The chain used two native invocations, no reviewer tool calls, automatic retries,
steer or fallback. The completed review reported 70,502 input and 2,077 output
tokens; host cost was not reported. No finding-resolution round ran.

## QwenCloud free text summary and continuation

On 2026-09-13, an authenticated, read-only quota lookup selected
`qwen3.8-max-0902`: valid, 1,000,000 tokens remaining before the campaign, with
[`Free quota only`](https://docs.qwencloud.com/resources/free-quota) enabled.
The test used a separate general API key and the exact international Responses
endpoint, with disposable configuration, workspace and sessions. The proxy read
the actual key from a private temporary file; Yo received only a synthetic local
credential, with no actual key in its configuration.

The first bounded run at `29fc2b9c` stopped after three of four requests. Two
ordinary responses completed, but summary collection rejected the first answer
at output slot 1 after reasoning at slot 0. No checkpoint was committed; the
interrupted summary's completion and usage remain unknown. Fix candidate
`8afc960f` binds the sole summary message by both output slot and item ID in
automatic and idle compaction, while rejecting other messages, extra content
parts, mismatched completion, duplicate completion and text after completion.

The changed-artifact run passed all four requests: initial answer, second Turn,
tools-disabled summary, and fresh-process `--continue` answer. Every response
returned HTTP 200 and completed with final usage. The exact initial reference
and `LEFT=red RIGHT=blue` facts survived the summary source, summary, stored
checkpoint, resumed request and final answer. The checkpoint body matched the
summary bytes, and the final sealed Journal message matched the response bytes.
Summary `resp_8473fe12-7673-9f6d-84fe-b37a6a479f79` and resumed answer
`resp_fb9e33e9-8724-9496-883c-4b889bfb9c4d` reported 7,954 and 1,510 total
tokens; the four completed responses reported 23,080 total. The campaign used
seven requests across its stopped original and changed-artifact runs, with no
redirects, automatic retries, other models or subscription fallback.

All 85 managed-backend tests passed, including output slots 0 and 1 through
automatic/idle compaction and disk resume, and seven invalid event cases in both
paths. Real-PTY offline success and HTTP-200 failure controls passed without
inference. Native Codex 0.154.0 independently accepted the exact implementation
candidate with no findings using `gpt-5.6-sol`, effort `high`, Session
`01a09945-2a3f-76c0-bf0e-2e86bfc24530`; one invocation, no reviewer tool calls
or finding-resolution rounds. Build, formatting and workspace Clippy passed.
The tested binary SHA-256 was
`78fa40e59ac859de2463b0474450cb35c8a5336dfd9bffccde66150ad2fa7c22`.

Both live TUI exits were zero and restored terminal settings. The proxy, owned
probe state, temporary key and review packet were removed; normal Yo/Codex
configuration and credential files were unchanged during the live test. This
passes the shared text summary/checkpoint/replay path. Image-aware summary
fidelity has separate OpenRouter and QwenCloud evidence on this page.

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

On 2026-09-13, the reusable Linux command
`python3 tools/validation/codex-policy-persistence.py /absolute/path/to/codex`
reconfirmed exact Codex `0.154.0` policy writes and restart reload in a
loopback-only user/network namespace. The host offered and accepted one exact
`acceptWithExecpolicyAmendment` argv prefix, wrote its native rule and executed
one owned marker append. After closing the app-server and starting a new
process, `thread/resume` and the identical command appended a second marker
without approval; the rule bytes were unchanged. A distinct command offered
`cancel`, which interrupted the Turn, left its marker absent and wrote no rule.
Five local synthetic Responses requests and zero external model requests were
used. `gpt-5.5` selects the native fixture's tool configuration only; no OpenAI
model service was called. Normal user state was unchanged and all owned test
state and processes were removed. This direct native probe supplements the
actual Yo TUI journey above and does not add a persistent command-denial choice.
The runner requires the offered `cancel`, its recorded response and the
`interrupted` outcome, and asserts exactly five fixture requests.

The separate command
`python3 tools/validation/codex-interview-resume.py /absolute/path/to/codex`
created two nonsecret pending questions on the same exact native version,
closed its process, started a new one and resumed the disk thread. The old Turn
was `interrupted`, and no pending question RPC was replayed. Its one local
fixture request made zero external model requests; owned temporary state was
removed and normal user state was unchanged. This validates the recovery
constraint behind the proposed interview feature, not implemented draft
persistence or transparent continuation of a dead request.
The runner drains events after the resume response for a full one-second quiet
period, within a three-second bound. Delayed question RPCs, protocol errors,
reader EOF and process exit fail the probe; only a live, quiet app-server passes.

### Automatic refusal and network approval scopes

On 2026-09-11, the same Yo binary and official Codex `0.154.0` passed ten
additional turns through an isolated 120×48 tmux TUI. A credential-free local
Responses fixture supplied deterministic tool calls and reviewer responses;
there were 20 local model/reviewer HTTP requests and zero real-service requests.
This verifies native policy execution, adapter behavior and visible TUI results;
it does not measure the service model's risk classification.

These deterministic fixtures alone do not establish actual-service risk
classification. The free Qwen native-host check below adds real classification
evidence for its exact route; no free OpenAI Codex service route was established.
This is separate from choosing an independent code-review model for a task.

On 2026-09-13, a separate native Codex `0.154.0` app-server probe used the
user-authorized renewed QwenCloud Token Plan. Its main-agent tool call was a
local fixture; the automatic review used an explicit custom-provider catalog
selecting `qwen3.8-max`. Offline allow and deny controls matched the exact
review, thread, Turn, command and independently checked file outcome, with
zero service requests.

The live allow case forwarded one unchanged reviewer request to the exact
configured Qwen Responses endpoint. HTTP 200 was observed, but no completed
assessment or final usage was established. A second local reviewer request was
blocked without forwarding. The native host failed closed with
`decisionSource: "agent"`, `denied`, high risk and unknown authorization, followed by the
correlated `declined` command; the marker file remained absent. This is failure
handling evidence, not a successful Qwen risk classification. No additional
request was forwarded to any service, and the live deny case did not run. Temporary
native state, dummy local authentication, scripts and processes were removed;
ordinary Codex configuration was unchanged. This direct native check does not
establish a new Yo TUI automatic-review journey.

#### Free Qwen native automatic-approval classification

The remaining-work run at `06d3b4db` selected general-API
`qwen3.8-max-0902`, with valid free quota and `Free quota only` enabled.
Official Codex `0.154.0` used an explicit custom-provider catalog and reasoning
effort `low`. The main agent was a local synthetic fixture; only unchanged
guardian requests reached
`https://dashscope-intl.aliyuncs.com/compatible-mode/v1/responses`.
This is service compatibility validation, not independent code/document review;
those reviews continue to use the configured authenticated Codex host.

The real allow assessment produced an agent-sourced `approved` review, low
risk and unknown authorization. It preceded the same thread/Turn/target command
completion with exit code 0, and the append file contained exactly one marker.
The first observer assumed a space after SSE `data:` and missed its completion
and usage; it also stopped the later local main-agent response. That Turn failed
at the local harness boundary after the approved command completed. The native
assessment and file outcome are evidence of classification; this request's
final usage and complete Turn accounting remain unknown. It was not resent.

The original deny probe completed a read-only inspection tool response, rather
than an assessment: `resp_3abc1b14-3b0a-91d7-a682-37dae8b43400`, 5,436 tokens.
Its one-request bound blocked the native follow-up; the resulting failure
denial is excluded from genuine classification. After accepting legal `data:`
fields without a space and validating native inspection continuation offline,
a separately frozen changed-artifact deny check allowed two requests. Its
first response, `resp_43e22ea4-893c-94f4-bea2-2de42c8f74eb`, completed the
inspection with 5,467 tokens. The follow-up
`resp_859432c7-3ee1-98dd-929b-eed041447ae5` completed with 5,728 tokens and
an actual `deny / high / unknown` assessment. The agent-sourced native review
`ab30b414-c020-4229-a932-a8ce5fe84ad0` correlated to the same thread, Turn and
command before its `declined` completion; that command did not execute, and
the Turn completed. The forbidden upload used a dummy credential file and a
reserved invalid hostname pinned to loopback, with no real secret or remote
upload target.

This supplies genuine allow and deny classification on the tested Qwen route.
It used four service requests across the original two-request scope and the
changed two-request inspection scope: 16,631 known tokens across three
completed usage reports, plus the allow request's unknown usage. The second
request of the final check was a native read-only-tool continuation, not an
HTTP or assessment-error retry. No redirects, HTTP retries, model change,
subscription fallback, manual approval or saved rules occurred. Offline
allow/deny and inspection controls passed without service requests. Temporary
native state, local dummy authentication, scripts, listeners and processes
were removed. This direct native probe does not claim a new Yo TUI journey or
risk accuracy on other models, providers or actions.

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
preedit/commit and terminal Command-V were covered by the
[current direct input verification](#current-mac-direct-input-verification).

## Current Mac direct input verification

On 2026-09-15, accepted `develop` commit `35d3e47f` built with the locked
dependencies on the configured arm64 Mac. In a dedicated tmux pane, the user
completed physical Korean/English switching and IME composition, Backspace and
direction-key editing, a two-line Korean/ASCII/emoji paste through the terminal
application's actual Command-V, and `Ctrl+C` draft clearing without reporting
an input fault.

The post-exit observation was repeated after two result-wrapper faults outside
Yo: the first pane did not retain its markers, and the next zsh wrapper assigned
the shell's read-only `status` variable after Yo returned. A Bash
result-preserving run recorded empty-`Ctrl+D` exit status 0, alternate-screen
release and exact shell termios restoration. The tested binary SHA256 was
`e25e97801ad161e930335ae669079913fd79870f4351149a3b4bc960bbc3860b`.

The stock Fullscreen TUI used an offline Provider and isolated config, Codex and
Session state under a native macOS sandbox that denied network access. The
Session repositories contained zero backend bindings, accepted requests and
finished Turns. This establishes physical terminal input and exit behavior; it
does not establish model-service behavior. The dedicated tmux session,
temporary checkout, build and state were removed. Installed Yo, normal
credentials, existing tmux sessions and clipboard setup were preserved.

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
