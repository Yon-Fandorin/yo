# Validation

Choose evidence by the boundary that changed. Start with the smallest check
that can distinguish the expected behavior from its important failure. Ordinary
work uses affected checks; the formal baseline below applies only to a selected
[Slice](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#slice-contract).

## Evidence layers

| Layer | What it establishes | Examples |
|---|---|---|
| In-process | Deterministic state, protocol, layout, rendering, and injected failure behavior | `yo-core` engine/runtime tests; `yo-tui` component tests; rendering parity goldens |
| Host-integrated | Behavior of real host facilities without optional installed services | Linux PTY, termios, process signal, and terminal-restoration tests in `yo-cli` |
| External environment | Compatibility with installed programs, authentication, and nested terminal environments | Codex, Grok, tmux, local `sshd`, SSH, and tmux inside SSH |

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
| Typed input spans, submission identity, or fixed-v1 structured-reference rejection | `cargo test -p yo-core input::tests` and `cargo test -p yo-core journal::codec` | `crates/yo-core/src/input/tests.rs` and Journal wire-compatibility tests |
| Agent-session admission, concurrency, startup, or shutdown | `cargo test -p yo-core agent_session::tests` | `crates/yo-core/src/agent_session/tests` |
| Backend lifecycle, evidence, or bounded child-process transport extraction | `cargo test --locked -p yo-backend` followed by `cargo test --locked -p yo-core backend::evidence` and `cargo test --locked -p yo-core journal::codec::tests::correlation` | `crates/backends/foundation/src`, the `yo-core` specialization, and Journal wire/recovery compatibility tests |
| Codex protocol translation or provider-ID correlation | `cargo test --locked -p yo-backend-delegated-codex` | `crates/backends/delegated-codex/src/runtime/tests.rs` |
| Grok ACP translation, permissions, authentication, or Session correlation | `cargo test --locked -p yo-backend-delegated-grok` | `crates/backends/delegated-grok/src/runtime/tests.rs`, `observation/tests.rs`, and `protocol.rs` |
| Decoded input, editing, paste, bindings, or exit gestures | `cargo test -p yo-tui input::` | Tests beside `yo-tui/src/input` |
| Prompt wrapping, cursor visibility, or viewport behavior | `cargo test -p yo-tui prompt::` | Tests beside `yo-tui/src/prompt` |
| `@` trigger, stale result, selection replacement, local ranking, or Git-ignore discovery | `cargo test -p yo-tui workspace_reference` and `cargo test -p yo-core workspace_reference` | `yo-tui/src/prompt/workspace_reference.rs` and `yo-core/src/workspace_reference` |
| `$` trigger, Codex catalog decoding, scope filtering, disabled rows, or typed skill selection | `cargo test -p yo-tui skill_reference`, `cargo test -p yo-core skill_reference`, and `cargo test -p yo-backend-delegated-codex skill_catalog` | `yo-tui/src/prompt/skill_reference`, `yo-core/src/skill_reference`, and `backends/delegated-codex/src/skill_catalog.rs` |
| Transcript items, streaming revisions, or scrolling | `cargo test -p yo-tui transcript::` | Tests beside `yo-tui/src/transcript` |
| Shell composition, layout, Surface, Unicode width, or text flow | `cargo test -p yo-tui` | Tests beside the owning `yo-tui` module |
| ANSI operations or presentation-mode policy | `cargo test -p yo-tui terminal::` | Tests under `yo-tui/src/terminal` |
| Inline or Fullscreen mode behavior | `cargo test -p yo-tui terminal::mode::` | Tests under `yo-tui/src/terminal/mode` |
| Live-loop ordering, backpressure, submission draft ownership, or event projection | `cargo test -p yo-tui runner::` | Tests under `yo-tui/src/runner` |
| Terminal and HTML projection of the same completed frame | `cargo test -p yo-tui --test rendering_parity` | `crates/yo-tui/tests/rendering_parity` and its goldens |
| Process termination or real terminal restoration | `cargo test -p yo-cli pty_tests::` | `crates/yo-cli/src/pty_tests/` |
| Unix process-coordinator state and compensation | `cargo test -p yo-cli execution::process::termination::tests` | `crates/yo-cli/src/execution/process/termination/tests` |
| Shared bounded YAML parsing, inference, and failure budgets | `cargo test -p yo-yaml` | `shared/yo-yaml/src/lib.rs` |
| Required explanations immediately above Rust tests | `cargo xtask check test-explanations` | Rust sources under `crates/`, `shared/`, and `tools/` |
| Task-context routing, changed-file documentation hints, or local reference checks | `python3 -m unittest discover -s tools -p test_context.py` and `python3 tools/context.py check` | `tools/context.py`, existing Markdown route tables, and their local link targets |
| Slice changes remain inside their bound local write-set | `cargo xtask check slice-scope` | One active Slice worktree; the planner first runs `cargo xtask slice-contract bind <contract.json>` |
| Two Slice contracts have a common current integration base and disjoint declared ownership | `cargo xtask check slice-parallel <left.json> <right.json>` | Direct Slices use `develop`; Wave Slices use their Wave branch |
| One clean Slice candidate has validation, review, risk, and approval evidence bound to the same identity | `cargo xtask slice gate <request.json>` | Returns exactly one next action without rerunning validation or review |
| A ready Slice needs an exact commit message and close record without identity transcription | Run `cargo xtask slice commit prepare <gate.json> <message-source> <message-out>`, commit the exact squash, then run `cargo xtask slice close prepare <request.json>` before `close plan/apply` | The first prepare runs in the clean Slice worktree; close prepare runs in the clean integration worktree after the accepted commit |
| Repository hook policy or structured development checks | `cargo test -p xtask` | `tools/xtask/src` |
| Prospective activation ContextBuild and review-packet identity | `cargo test -p methexis activation_review_context` and `cargo test -p xtask review_packet::tests::prospective` | Exact activation request, proposed Checkpoint/active record, authority mode, packet replay, and active-authority cross-use rejection |
| tmux, SSH, or nested tmux behavior | See the [terminal environment matrix](./terminal-matrix.md) | Ignored `yo-cli` environment tests |

These commands are entry points, not permission to ignore affected neighboring
boundaries. For example, an edit to `AgentSession` can require both its focused
tests and the TUI runner tests when the admission result observed by the
frontend changes.

For model-connector request and stream validation, run the concrete Connector
crate that owns the changed dialect, such as
`cargo test --locked -p yo-connector-openai-chat-completions` or
`cargo test --locked -p yo-connector-kimi`, together with
`cargo test --locked -p yo-connector-transport` when shared byte lifecycle
mechanics are affected. Run `cargo test --locked -p yo-core` at close for the
neutral vocabulary and managed-loop consumer. Environment-integrated Connector checks use only local
`127.0.0.1` HTTPS listeners and require the external `python3` and `openssl`
commands to create and serve their ephemeral test certificates. Missing
prerequisites fail the command rather than skip its assertions; record the
host/platform, prerequisite versions, and pass/unverified result for each
validation run.

## Live skill selection and resume

With an authorized authenticated backend, create a disposable workspace and a skill at
`.agents/skills/yo-proof/SKILL.md`. Give it `name: yo-proof`, a description, and an
instruction to reply with a fresh random token that appears only in the skill body.
For managed backends, configure that workspace's `.agents/skills` as a `skills.roots`
entry with `scope: workspace`. Codex uses its own authoritative skill catalog.

In the actual TUI, type `Use $yo-proof`, wait for the enabled Skills result, and press
Enter to select it. Confirm the draft remains unsent; press Enter again to submit.
Require a committed `yo.structured-input/v2` StartTurn with the exact reference span,
projection, locator, scope, environment, revision digest, and full original instructions.
Check the completed assistant message for that turn equals the token.

Exit normally, remove only the disposable skill file, and resume the same Session in
a fresh process. Ask for the earlier token without including it in the new prompt.
Require a second completed turn with its own exact assistant answer and preserved
frozen input. Validate the visible archive through `yo session UUID --ascii` as well as
the typed journal. Durable messages can use separate segments or a final inline segment;
match their activity and revision, including superseded streamed revisions. Record
backend-specific results, cleanup, and any tools or interactive requests observed.

## Live managed command approval and recovery

Use an authorized backend with a fresh disposable workspace and repository. Request one
exact `run_command` that appends a synthetic marker to a file, decline it in the actual
TUI, and require the file to remain absent. Request a separate append command and approve
only its exact arguments, tool ID, effect, and digest. Resizing the approval panel must
not execute either command. The approved file must contain exactly one marker; append
rather than replacement makes duplicate execution observable.

Correlate each committed response with the request activity, turn and request ID, and
match the command, call ID, execution host and outcome across the approval text, typed
ToolOutput and replay result. A declined call returns a failed tool result to the model;
it does not itself interrupt the turn. Pending approval text is a nonterminal message:
read its latest durable segments rather than waiting for `message_ended` before responding.

At idle with an empty draft, Ctrl-D exits normally. If using `/exit`, select its displayed
command entry; Esc deliberately allows the unchanged slash text to be sent to the model.
Remove only the synthetic result file, resume in a fresh process, and ask for the previous
marker without including it or using tools. Check the exact answer, no new tool activity,
and preserved physical journal prefix. Keep failed driver records separate from a later
audit or continuation and account for every actual turn, including unintended test input.

For delegated Codex, use a disposable launch configuration with approval routing to
the user and an explicit command approval request. Do not change the user's persistent
policy. Parse the durable `yo.activity-approval/v1` profile and select the offered
`Approve request` ordinal for a one-request approval; do not select a saved rule or
session-wide grant. The structured panel accepts offered ordinals, not a generic `y`.
Its decline choice may be `Decline and stop`, which interrupts the turn and need not
produce a final assistant answer. Inspect the actual offered choices and turn outcome.
Match `commandExecution` arguments and its final typed result rather than expecting
managed FunctionCall replay records. Fresh-process resume must preserve the native
session locator and physical journal prefix. The same append, absence, resize,
tool-free recall and cleanup checks apply. This validates the explicit user-routed
approval path, not automatic approval review or persistent policy amendment.

## Chat visual previews

### Interactive test agent

Inside the actual `yo` chat, enter `/preview`. This opens an isolated, ephemeral
child TUI state with an offline test agent; no Python launcher or separate app
is involved. Type any text for streamed replies, or send `tools`, `error`,
`long`, `markdown`, `tables`, or `diff` for simulated tool output, failure/recovery,
scrolling, formatted prose/code, responsive tables, and code changes. These scenario
names are ordinary messages inside the sandbox. `Esc` interrupts. `/preview`,
`/exit`, or the empty-prompt exit gesture returns to the original conversation.
Entry is rejected while a real turn, submission, or request is pending.
The initial expanded host document groups examples by code/documents, files/search,
tools/terminal, approvals/interview, progress/status and charts/media. It uses the
normal document theme and wrapping; Alt+Up reaches its start immediately after entry.
The guide includes file/content search, shell variants and aggregate diff/reroute examples.

For a persistent standalone offline terminal, run
`cargo run --locked -p yo-tui --example chat_preview`, then enter `/preview`.
This uses the production TUI without a model connection or test-harness timeout
messages. Wait for `Enter send` and an empty prompt before sending each scenario.


The parent continues to own real observations and session output. Preview
commands and synthetic records stay on the child; frame preparation pins the
same appearance and disables preview scrollback publication. Preview draft,
transcript, and navigation are discarded on return. The original conversation
is preserved; the command text itself is consumed. Editing, paste, resize,
view navigation, and rendering use the production TUI. Busy submissions are
explicitly rejected with the sandbox draft retained; steering, approvals,
provider behavior, persistence, and real tool execution are not simulated.
An idle preview does not arm a periodic poll. Pending synthetic output supplies
its own deadline, combined with motion and real-agent backpressure deadlines.

Implementation: `command/preview.rs` registers the command;
`runner/state/preview.rs` owns isolation and lifecycle;
`runner/preview_agent.rs` owns synthetic event generation. Check with
`cargo test --locked -p yo-tui` and the neighboring `yo-cli` PTY tests.

Approval profiles may carry an optional `related_change`: a nonzero observed file-change
activity ID in the request's own Turn, independent of provider wire IDs. Missing fields
remain compatible with earlier profiles. During such an approval, `/changes` opens the
linked activity's first file, rather than a more recent unrelated change. If the declared
activity is unavailable, the UI explains that instead of selecting another file. Reviewing
changes never submits an approval; normal committed-frame guards still apply to decisions.
Codex supplies this reference only for an exact item ID observed as a file change in the
same Turn. If the request arrives first, a later started/completed item with a file-change
body refreshes the unanswered approval after publishing that body. Already answered
requests are not refreshed, and a matching update is emitted at most once. Admission
reserves enough encoded space for the largest possible activity ID, so a later link cannot
exceed the profile bound. Link-only updates and subsequent body/status revisions of the referenced file change
require a newly committed approval frame. A frame prepared before such a revision cannot
re-enable keyboard or typed-ordinal approval. Unrelated files and matching numeric IDs in
other Turns do not invalidate the request panel. This is a freshness gate, not proof that
the user inspected every retained diff row.
Completed file-change activities remain addressable in the retained transcript; the
Codex adapter retains their item mapping until the Turn ends.
`/preview approval-diff` demonstrates reviewing the exact proposed file despite a newer
unrelated change; all choices stay offline.

Approval requests retain reported command, working directory, environment, permission,
network, policy proposal, offered-decision, and unknown context fields. File-change
write roots are labeled as requested session scope. Bounded serialization rejects
oversized context before publishing an approvable request. `Approve request` uses
the reported scope; it does not promise one-time access. For explicit
`availableDecisions`, `ActivityApproval` (`yo.activity-approval/v1`) retains the
original choice order, scope descriptions, and a non-granting default. The UI places
decline/cancel first; without one, Enter defaults to interrupting the turn. Selection
and typed ordinals require a committed visible frame, including after profile updates.
`ApprovalDecision::Offered` records the original one-based ordinal and request identity
in the closed command codec, without a new SubmissionId. Codex resolves that ordinal
against the outstanding request: accept, session acceptance, decline, cancel, exact
command-policy amendment, or exact persistent network allow/deny rule. File-change
requests cannot submit command/network policy objects. Unknown or malformed decisions
remain disabled, and out-of-range ordinals never produce a wire response or receipt.
If choice text cannot be rendered safely, the panel offers only interruption;
typed ordinals cannot bypass that fallback. At most 64 choices and 16 MiB of encoded
profile are admitted; failure rejects the
request rather than substituting generic approval controls. An absent/null decision
list derives the native Codex defaults verified in 0.145.0, 0.146.0, and 0.149.0.
Network requests offer accept, session acceptance, the first proposed persistent allow
rule (if any), and cancel. Otherwise, additional permission requests offer only accept
and cancel; ordinary commands additionally offer their proposed command rule, if any.
File changes offer accept, session acceptance, and cancel. Explicit lists, including
empty lists, always override these defaults. Invalid hints used to derive a policy
are rejected before binding. Legacy binary API accept/decline replies remain unchanged
when the server supplied no explicit constraint; UI ordinals resolve only against the
derived list. See the pinned [command defaults](https://github.com/openai/codex/blob/rust-v0.149.0/codex-rs/tui/src/approval_events.rs)
and [file approval menu](https://github.com/openai/codex/blob/rust-v0.149.0/codex-rs/tui/src/bottom_pane/approval_overlay.rs).
Managed and Grok adapters reject offered
ordinals they do not support before consuming their approval state. A decision receipt
is published only after the response write succeeds, using customizable activity styles.
The offline `approval-scopes` fixture exercises scoped choices without changing any
permission, policy, or file. Live provider policy persistence remains unverified here.

### Static comparison fixtures

`transcript::layout::markdown` parses only assistant message content into styled
cells: headings, nested emphasis, links with visible destinations, lists with hanging
indents, quotes, task checkboxes, and literal code blocks. Prose wraps at spaces;
long tokens fall back to grapheme wrapping. Code panels also keep fitting words together,
but retain every whitespace glyph and original source offset, including indentation
and expanded tabs. Oversized tokens still split at grapheme boundaries. Code panels use a dedicated body background and a distinct language-header band.
Plain-text fences (`text`, `txt`, `plaintext`) omit the redundant label row while
retaining the body background, padding and literal contents. Programming-language
and diff labels remain visible. `tui.code_padding` / `OutputPreferences::with_code_padding`
sets horizontal panel padding (default 1, zero allowed, capped at 8). Narrow panels
reduce padding to leave readable body columns. Assistant Markdown, host/tool documents
and literal file diffs share it; theme changes and source exports preserve the preference
and original content respectively. The Rust preview accepts `--code-padding=N`.

Diff additions and deletions fill the entire row, including inner padding,
and soft-wrapped continuations. Dark tints, light pastels, and color-free mono
are resolved centrally; code bands honor body width and scrolling.
GFM tables retain cell emphasis, CJK widths, and left/center/right column alignment.
If natural column widths exceed the available body width, `markdown/table.rs`
shrinks columns and wraps each cell while preserving alignment and styles. A binary
search finds the shared width ceiling, with spare cells assigned to earlier columns;
resizing does not repeatedly scan all columns for each removed cell. The table
regression covers unequal widths, tied columns, and 32 long cells across resizes. When
columns cannot retain eight cells (or their shorter natural width), it presents
labeled fields separated by a blank line; values are never truncated. Cell tabs and controls are normalized before alignment. `diff`/`patch` fences and typed file-change activities use addition/deletion/metadata theme roles; literal signs remain in
all palettes. File headers require the separator after `---` or `+++` so changed
content beginning with repeated signs is not mistaken for metadata.
GitHub-style footnotes render resolved `[^name]` references and definitions with
matching `[name]` labels. Definition bodies retain Markdown links, inline code and
nested lists under an indented body. Labels use the existing bold quote role; body
width and theme preferences apply. Definitions stay at their source position and
references do not imply an internal jump action. Unknown references and fenced code
remain literal; a definition arriving in a later snapshot resolves the reference.
The `footnotes` preview includes repeated and Korean references.
User text, tool logs, notices, stored records, and plain output keep source syntax.
Approval/interview requests, answer/decision receipts, and typed notices wrap at word
boundaries without parsing markup. Literal prose retains explicit indentation and
hard line breaks; oversized words split only at grapheme boundaries. The shared
`text/flow.rs` prose engine also serves Markdown, whose leading-space policy remains
unchanged. Raw log placement and editor cursor coordinates retain their literal
flow. Tests cover UTF-8 source offsets, combining characters, narrow CJK/emoji,
indentation, source export, and themed request/response frames.
Tests cover incomplete streaming fences, CJK/emoji boundaries, control notation,
style spans across wrapping, and source-output preservation. The event-only
`pulldown-cmark` dependency has no HTML/CLI features or terminal/platform calls;
`markdown.rs` isolates parser replacement. Linux compilation and rendering are
checked here; macOS runtime behavior still needs the platform matrix.

Tool calls, tool results, and file-change observations carry typed presentation
status. Titles, literal logs, and outcome footers receive separate theme styles;
log text resembling a failure never selects an error style. `transcript/activity.rs`
publishes terminal status, the owned footer boundary, and Final phase in one
revision, rejecting overflow without partial mutation. Turn failure/interruption
notices use the same status roles. Tool-call interruption keeps the status heading
without appending a duplicate interruption line. Tests exercise actual event-to-frame
styles, fake failure text in logs, source preservation, and immutable finalization.

Codex's `runtime/events.rs` retains command metadata from `item/started` before
output deltas, then replaces it with the authoritative completed snapshot including
reported exit code and duration. File changes retain paths, structured rename kinds,
and literal diffs; add/delete payloads carry full file content and are converted to
signed lines (including missing-final-newline notation), while update diffs stay
unchanged. See [Codex's display conversion](https://github.com/openai/codex/blob/main/codex-rs/tui/src/app_server_approval_conversions.rs).
MCP/dynamic calls retain identity, arguments, results and errors.

Codex `imageGeneration` items now retain their tool lifecycle, prompt, saved-path
metadata, failures and generated PNG in `ToolOutput`. The complete original item
is available in the first content block's `source` for custom tool renderers;
image bytes use the existing common media renderer and image preferences. Empty
results have an explicit no-payload state. Saved paths are never opened by this
presentation adapter. Existing `mcp-image` previews exercise the same media path.

Codex `imageView` also retains its started/completed tool activity and original
path in `ToolOutput`. Its item carries no image bytes, so the view explicitly says
so and keeps the path literal. Custom tool renderers receive the complete original
item in the first content block's `source`; unknown metadata remains readable.

Codex collaboration tool items use the same `ToolOutput` contract. Tool completion
and each reported agent state remain distinct: a completed wait can still list a
running or failed child. Sender/recipients, requested model/effort, prompt, child
messages and unknown metadata remain available, including the original item for
custom tool renderers. The collaboration-specific interrupted status maps to an
interrupted activity. `/preview agent-tasks` is an offline example of these states.

Codex review-mode entry/exit items retain their lifecycle as ModelWork documents.
The reported review text stays Markdown under “Review started” or “Review ended”,
using the existing DocumentRenderer, code styles, folding and source export.
An empty exit is not a clean-review verdict; malformed or oversized profiles retain
literal source. This maps received events; it does not add a `review/start` command
or request an external review.

Codex `subAgentActivity` notifications update one ModelWork notice per item ID;
the reported child kind, path and thread remain distinct from the parent activity's
completion. Unknown metadata stays literal. `sleep` uses a common ToolOutput and
labels `durationMs` as the requested duration, preserving unsigned integer precision
including zero. It does not infer elapsed time or a countdown. Original sleep items
are available to custom tool renderers; notices use the existing semantic palette.

Codex recorded `functionCallOutput` items use ToolResult activities. Their canonical
start/completion shares one identity, with qualified function name and original item
retained in ToolOutput for customization. Text remains literal; input_text/image/audio
blocks normalize into the existing common media shapes without overwriting colliding
fields. Empty output is explicit; malformed bodies and unknown blocks remain literal.
This records a received function result and does not dispatch a new tool invocation.

`hookPrompt` is recorded as a distinct “Hook context” ModelWork document, not as a
new user submission. Hook-run IDs, fragment order, exact text and unknown metadata
remain visible; a fence longer than any source backtick run keeps the context
literal. Empty fragments have an explicit hint; malformed or oversized profiles
keep their original source. Existing DocumentRenderer, folding and code preferences
apply. The upstream context parser and full transcript rendering distinguish this
hook-provided input context from ordinary user messages.







`runtime::tests::coding_events` checks the adapter's actual event order and payloads.
No provider call is needed for those tests. An installed initialize smoke proves
only handshake and cleanup, not a completed coding turn or full version admission.

The opt-in `live_agent_session::local_codex_completes_a_real_file_change` integration
probe uses an authenticated Codex model turn in a disposable directory. It runs `pwd`,
creates one fixed-content file with the patch tool, and requires completed ToolCall and
FileChange activities, a matching added diff line, completed Turn and exact file bytes.
The temporary directory is removed afterward. Run it with
`cargo test --locked -p yo-backend-delegated-codex --test live_agent_session local_codex_completes_a_real_file_change -- --ignored --exact`.
Admission retries retain the same PendingCommand for at most 30 seconds; a rejected
submission fails immediately. Only an admitted command starts the 180-second completion
wait. Ignoring Backpressured can otherwise produce a false timeout with zero activities.
Unexpected interactive requests fail without sending approval. Passing this probe proves
the bounded real coding path, not TUI interaction, approval choices or all provider features.

The opt-in `local_codex_resumes_a_durable_session_and_remembers_prior_input` test in
that same target performs two text-only model turns separated by backend shutdown,
repository reopening and durable continuation recovery. The second prompt omits the
random nonce from the first turn; the final reply must reproduce it exactly. The test
also requires the same Session, descriptor, binding and locator, the next Turn ID,
and an unchanged durable record prefix. It uses a disposable repository and a read-only
host profile, rejects unexpected tool or interactive activities, and checks that the
workspace stays empty. Startup, admission and each turn have explicit time bounds.
Run the same command above with this test name. This proves native continuation for
the installed Codex environment; it does not establish managed replay or compaction.

`backend::tests::context_replay::automatic_compaction_survives_disk_resume_with_exact_retained_connector_input`
in `yo-backend-managed` covers automatic compaction across actual local storage and
restart. Three ordinary turns trigger one summary; the successor connector checks
that the checkpoint is already readable from disk. After shutdown and repository
reopening, a fresh backend must issue no request during recovery and exactly one
request for the fourth input. The test compares its complete ordered context, binding,
context epoch, and unchanged stored record prefix. Its recording connector and token
counter are controlled fixtures; this is not an authenticated provider or tokenizer test.
For an authenticated pressure probe, use a fresh disposable workspace and synthetic
history. Put a unique recall token only in the old turn to be summarized, retain a
separate short turn, then submit enough new input through the Rust TUI to cross the
configured trigger without `/compact`. Check one automatic checkpoint, exact retained
turn/current-input bytes and their order, and a reduced admitted request size. Shut down
gracefully and ask for the token in a fresh resumed process without including it in the
question. Verify it is present only in the summary of the effective pre-recall context,
while the physical journal retains its original prefix. Preserve the configured counter
profile: conservative UTF-8 byte counts are not provider-reported token usage. Journal
sequence checks complement the controlled disk-before-dispatch test above; they do not
independently measure fsync timing. When driving tmux, preserve newlines with
`paste-buffer -p -r` and compare the actual committed input, not only the intended prompt.

Summary requests carry visible history as one JSON user message, preserving roles and
tool-call relationships as data. Both automatic and explicit compaction exclude private
replay and disable tools; ordinary replay remains provider-native. The owning tests check
the actual connector input and the encoded 16 MiB source limit, including JSON escaping.
A completed, validated idle summary that admits the payload but does not reduce its
size reports a compaction rejection and keeps the original context available for the
next input. Core worker tests cover rejection both during command dispatch and after
summary polling. A fully completed idle response with valid usage but an invalid portable
summary is also rejected without replacing context; the validator stays strict and no
automatic retry is sent. The notice contains a static format diagnostic, never the model
body. Pressure, incomplete/protocol responses, missing usage and cleanup failures remain
fail-closed. Automatic compaction keeps its existing failure behavior.
The managed disk regression shuts down immediately after rejection and checks the
recovered binding, epoch, Anchor and full replay before continuing. A standalone
`CompactContext` record preserves the prior Anchor; a later ordinary accepted request
still invalidates it until a matching completed outcome and Anchor are durable.

Typed file-change activities use `transcript/layout/activity.rs` and the existing
literal diff renderer. Source fences cannot escape into Markdown or image parsing.
Added/removed line counts are presentation-only. Folding moves background rows
with glyphs and retains the terminal outcome. `changes` in `/preview` exercises
this activity path, while `diff` remains a Markdown fence example.

`/changes` opens a read-only projection of retained typed FileChange activities,
not a Git working-tree scan. Left/Right selects file sections, navigation keys and
wheel scroll uncollapsed content, and F1 restores Chat. Explicit file metadata
owns section boundaries even when its update contains `diff --git` headers.
The fixed header shows the selected explicit file path during scrolling. At narrow
widths it prioritizes the file index and path, abbreviating the leading path with
an ellipsis when necessary; suffix graphemes use the Surface cell-width profile.
The full path and diff source remain unchanged. If a grapheme cannot fit (for example,
CJK at one column), the page explicitly switches to escaped text, retaining line
breaks and diff roles. Escape spans map back to original source offsets, so resizing
restores the original characters and reading position, including navigation inside an
escape sequence. The shared read-only page constructor maps escaped rows to original offsets;
its temporary span mapping is discarded after construction. The `changes` preview
includes CJK and emoji to exercise this boundary. Raw Git headers retain the generic
heading rather than guessing a path from potentially quoted filenames.
`runner::tests::views::navigation` checks narrow headers, file selection, scroll,
Chat restoration, and that review input cannot submit a model request.
The selected file now uses `TextPages` with usize row counts and a cached
item/section/revision/width layout, painting only the visible page. Original logical
line offsets preserve addition/deletion/hunk styles across wrapping, including row
backgrounds and padding; the same diff role classifier serves Markdown and review.
Theme changes resolve fresh styles without rebuilding source pages. End follows new
snapshots, detached navigation anchors to source bytes across width changes, and a
file switch starts at its heading. Chat displays a large-diff summary and `/changes`
hint before an individual diff exceeds its inline u16 height budget. This applies even when inline
activities are expanded; retained source/export and full review stay available.
Tests cover 70,000 lines, failed-frame navigation retry, appended snapshots, file
switches, continuation backgrounds, custom colors and width round trips.

Accumulated Chat/Transcript/Request layout and scrolling use usize document rows.
Glyphs, code bands, user backgrounds, context-item lookup and raster placements map
only visible logical rows into the unchanged u16 Surface coordinates. Natural live
height is also measured in usize and clamped to the actual terminal height at the
inline live composition boundary. Arithmetic overflow still produces typed errors.
Tests cover totals of 65,535, 65,536 and 98,308 rows, Home/End and positions near
usize::MAX, expanded 80,000-row Chat diffs, code/diff/image placement after long
history, and a large unpublished inline suffix. This removes the combined-message
height ceiling; it does not remove individual rich-message layout limits or make
one persistent inline publication Surface exceed u16. Oversized publication remains
an explicit preparation failure before cursor acknowledgement. Layout still prepares
all retained items; fully virtualizing individual message bodies remains separate work.

Codex `turn/plan/updated` replaces one ModelWork activity per turn and closes it
before turn completion; late plan updates cannot reopen it. Remaining steps are
never marked completed by inference. `plan` previews the same snapshot projection.
`item/tool/requestUserInput` presents nonsecret questions sequentially and sends
one answer map on the original JSON-RPC ID after the final answer. Numbered choices
map to labels; free-form Unicode answers are preserved. Core-runtime tests cover
successor request correlation, duplicate responses, partial-answer interruption,
and late resolution. Invalid/secret questions are rejected without showing their
contents. This does not add secret entry or image model input.


`runner/chat.rs` reuses `SessionUsageProjection` to validate one usage snapshot;
its compact footer is published only after the corresponding activity completes.
The `Last` values describe the latest reported observation, not session totals,
remaining quota, current context occupancy, or a cost estimate. Unsupported and
absent values remain explicit. Malformed known receipts show an unavailable state;
raw committed records stay unchanged. `usage` and `mcp` are offline preview cases.

For activity regressions, `runner::tests::activity_projection` checks that
completion, failure, and interruption clear the `Working` row and motion demand,
and that terminal tool headings replace progress labels without changing payload.
`terminal::mode::fullscreen` tests frame-batched ANSI output and recovery after
partial writes or failed flushes. Fullscreen frames wrap both the diff and final
cursor in synchronized-output begin/end sequences (CSI ?2026h/l), then write the
batch. This keeps intermediate label updates hidden on supporting terminals;
batching alone is not a guarantee of atomic display. A write/flush failure attempts
to release synchronization without replacing the original error or committing the
frame. Unsupported terminals and persistent I/O failures remain host limitations.
The inline renderer is unchanged by this fullscreen-specific refinement.

The user-requested Rich spinner refinement uses `⠋ ⠙ ⠸ ⠴ ⠦ ⠇`, keeping
three dots and rotating one perimeter position per frame, including wraparound.
Each step is 133,333,333 ns (approximately 800 ms per revolution); ASCII keeps
80 ms. `appearance::tests` verifies dot masks, cadence boundaries, and cell width.

Marker transitions have their own deadlines, independent of the 16 ms sheen
tick. The scheduler reserves the next marker slot using the FPS interval and
last observed render cost, merging nearby ordinary redraws into that slot.
`runner::unix::timing::output_timing` checks real ANSI writes against a virtual
clock at 60/120fps, with 7 ms input requests and fixed 0/2 ms write costs.
Variable host or terminal latency can still introduce visible jitter.

The shell `Working` label pulses uniformly as a whole in TrueColor; its position
and font weight remain fixed. Limited/Unknown color modes use static label ink.
The marker rotates at constant brightness. `shell::chrome::tests` checks uniform
label color and stable geometry/weight over two cycles, including narrow rows.
The runner starts a new visible turn at its first marker and keeps that epoch
across redraws within the terminal generation. `timing::motion_tests` checks
identity transitions; `runner::tests::reentry` checks the first presented glyph
with an old generation timestamp. Input timing retains the generation clock.
These local refinements differ from the accepted motion contract below;
selection-panel title sheen remains unchanged.

For comparison, pi's [default Loader](https://github.com/badlogic/pi-mono/blob/main/packages/tui/src/components/loader.ts)
uses ten frames at 80 ms and increments the index on each timer callback.
Yo intentionally keeps six frames and elapsed-time selection, so late wakes can
skip phases rather than replay them. Both pi's [main-screen renderer](https://github.com/badlogic/pi-mono/blob/main/packages/tui/src/tui-main-screen.ts)
and yo's fullscreen renderer use synchronized output; this does not establish
identical visual motion or prove that host-level glyph jitter is resolved.

This worktree change supersedes the old ten-frame choice for this request;
the accepted Methexis checkpoint still describes the old profile and has not
been reactivated. Authority reconciliation remains required before integration.

For a terminal feedback loop, run `python3 tools/chat_preview.py` from the
checkout. It builds on first use and opens an alternate-screen viewer without
calling a model. Keys: `1` welcome, `2` conversation, `3` working, `4` Markdown, `5` tables, `6` diff, `7` tools, `8` failure, `9` interruption, `[`/`]` previous/next scene (including draft, history, commands, long tools, and multiple turns), `w` width,
`c` palette (default, light, mono, indexed light, ASCII), `b` rebuild, `r` reload, `s` snapshot, `q` exit. The viewer
requires at least the selected width and 28 rows; it reports smaller geometry
instead of painting a clipped fixture. It restores terminal settings on normal
exit or a Python exception.

Keep that viewer open while changing code. From another pane or an agent, run
`python3 tools/chat_preview.py --build-only`: after a successful renderer test,
the viewer automatically reloads the published generation while preserving its
selected scenario, width, and palette. Failed or incomplete builds never replace
the last successful generation. Logs, immutable generations, and snapshots live
under ignored `target/chat-preview/`; `--directory` selects another local output
directory. A snapshot contains `frame.ansi`, `frame.html`, and `frame.json` with
the exact generation and selected fixture, so feedback can identify one screen.
Artifacts are retained until manually cleaned; this tool never controls tmux,
sends keystrokes to another process, or accepts live chat input.

Validate the feedback tool itself with
`python3 -m unittest discover -s tools -p test_chat_preview.py` (Unix PTY required).

Run `YO_TUI_PREVIEW_DIR=/tmp/yo-chat-preview cargo test --locked -p yo-tui chat_preview`
and open `/tmp/yo-chat-preview/chat.html`. The test exports real Session-to-Surface
frames for empty, conversation, working, Markdown, table, diff, tool completion,
failure, interruption, long draft, detached history, command selection, long tool log, multi-turn, approval, interview, plan, syntax highlighting, charts, images, and media fallback states at 20, 40, and 88 columns,
with default, light, and mono palettes, indexed light, and ASCII/unknown-color
fallbacks. Light fixture cards use a light host background. Individual HTML files make
focused screenshots easier. The surrounding browser card is fixture chrome,
not terminal UI; its default background models one host theme.
Matching `.ansi` files use the production `FrameDiff` → `TerminalOps` →
`AnsiEncoder` path and can be replayed in a cleared terminal at least as large
as the fixture. They contain absolute cursor positions: use an alternate-screen
viewer that restores terminal state on exit. This is static frame playback,
not a live agent or a public `yo preview` command.

Inspect request-band contrast, quiet answer text, prompt rules, and key hints.
Visual references are the official [Claude Code terminal refresh](https://www.anthropic.com/news/enabling-claude-code-to-work-more-autonomously)
and [Cursor CLI Ask mode](https://cursor.com/changelog/cli-jan-16-2026)
screenshots: borrow restrained emphasis, clear input boundaries, and nearby
action hints, not unsupported controls or their branding. Wide idle frames
expose `@ files`; narrower frames fall back to essential keyboard help.
The test also checks that welcome and placeholder copy never enter conversation
output. These previews do not establish real-terminal lifecycle or compatibility
with every host palette; use the terminal evidence layers above for those claims.

### Whole-chat visual audit

Review the complete conversation, including transitions and controls, rather than
judging isolated Markdown examples. These are inspection routes, not an accepted
design contract or proof that every physical terminal has been tested.

| Elements | Inspect |
|---|---|
| Welcome, empty input, identity and workspace | Invitation to type; metadata stays quieter than the response |
| User marker, request band, wrapped user text | Clear turn boundary and aligned continuation rows |
| Answer marker, paragraphs, headings, emphasis | Readable hierarchy without turning ordinary prose into a title |
| Lists, checklists, quotes, links | Hanging indentation, visible destinations, CJK and combining characters |
| Inline code, fenced code, language labels | Literal whitespace, complete backgrounds, partial streaming fences |
| Syntax highlighting | Rust/Python/JSON tokens, multiline comments, unknown-language fallback, literal whitespace and code backgrounds |
| Charts and images | Negative/zero values, common zero axis, exact numeric labels, invalid data, PNG/JPEG limits, monochrome density, resize and scroll |
| Tables and diffs | Narrow-width fallback; additions/deletions remain identifiable without color |
| Tool title, logs, completion, interruption, failure | Status follows typed events; long logs fold; real failure details remain visible |
| Long conversation and history position | Detached position is visible; End restores live follow; working status remains visible |
| Empty, multiline and scrolled input, cursor | Long drafts retain conversation space; range appears on the lower rule; text is unchanged |
| Prompt rules, progress row, keyboard help | Stable idle/active geometry; send/interrupt outrank optional mode labels |
| Command, file and model selection surfaces | Focus, disabled entries, truncation, close/accept hints; open menus never advertise send |
| Approval and agent questions | Default decline; explicit request approval; correlated replies; follow-up question; cancel; no selection before a committed frame |
| Default, light, mono, limited colors, ASCII | Semantic roles remain readable using supported colors and attributes |

Arrow inputs received between frames are applied in order. Test bursts, boundary
reversals, failed-frame retries, and width changes against individual key presses.
Code panels use a language-header band and inner padding, without decorative
rails, corners, or continuation arrows. Right padding shrinks at narrow widths;
source text and diff signs remain intact during wrapping and scrolling. Wrapped code
retains source indentation, capped at one quarter of the available body width,
without inserting characters into the source. Fullscreen captures SGR mouse wheel input
(three rows per event); clicks and horizontal reports are consumed without editing.
Inline mode retains native terminal scrolling. With tmux `mouse on`, wheel events
reach the app; tmux copy mode remains a separate native scrollback view. Verify
mouse capture release on exit, panic, suspend, and partial entry failure.

Long Chat tool logs collapse after eight rendered rows. File-change panels keep
six opening rows (path, hunk and initial changes) plus the tail, and collapse
after twelve rendered body rows. The preview retains
opening and latest rows plus an explicit hidden-row count; Ctrl+O toggles full
live Chat output. Failure footers, retained records, plain output, and published
native scrollback remain complete. `long-tools` in `/preview` exercises streaming.
Long drafts use the prompt viewport (roughly one third of shell height, with a
small-screen minimum allocation limit); the cursor remains visible. Neither the
input range nor the history hint enters stored conversation text.

For comparison, inspect the official [Codex CLI interface](https://learn.chatgpt.com/docs/codex/cli),
[Claude Code interaction and transcript controls](https://code.claude.com/docs/en/interactive-mode),
and [pi interface and tool expansion](https://github.com/earendil-works/pi/tree/main/packages/coding-agent).
They provide references for restrained metadata, context-sensitive controls, and
detail disclosure. Apply those principles within yo's teal/slate appearance;
controls must correspond to behavior actually implemented in yo.

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

## Keep agent-facing output bounded

Run verbose validation through `tools/validation/bounded-run.sh` when its
output will return to an agent context. The wrapper preserves the command's
exit status and complete combined output under the worktree-local
`.local-exclude/validation-runs/` directory. A successful run returns one JSON
summary line. A failed run returns the same summary and at most the final 16
KiB of diagnostic output; inspect the complete local log only when that tail
does not identify the owning failure.

By default the summary schema is the frozen
`yo.validation-run-summary/v1alpha2`. It records the
launch `HEAD`, whether the worktree was clean, a boundary-aware hash and count
of the exact command arguments, the complete log's byte count and SHA-256, and
the `reviewed-descendant/v1` reuse policy.
This makes a clean candidate's result self-binding when the Slice gate compares
it with the declared command. A dirty summary remains useful for local
diagnosis but is not candidate evidence. The summary always reports
`"reused":false` because it records an actual execution; it does not discover
or reuse an earlier run automatically. A later gate may declare
`"reused":true` only for a passing summary with the same exact command when
trusted Git proves that its clean launch HEAD is an ancestor of the reviewed
final candidate. Frozen `yo.validation-run-summary/v1` and `v1alpha1`
artifacts remain gate-compatible with their original meaning; v1alpha1 does
not permit reuse.

For a command whose result is determined entirely by local repository bytes,
add `--reusable-local`. This opt-in emits
`yo.validation-run-summary/v1alpha3` with the
`reviewed-descendant-context/v1` policy. Besides the v1alpha2 bindings, it
records the target OS, architecture, and a Rust/Cargo toolchain fingerprint.
At a later reused gate, Yo observes those values again and fails closed if they
changed. Its `external_state:"none-declared"` assertion excludes commands that
depend on a network, clock, account, service, or other external state; rerun
such commands instead. The option does not search for an earlier receipt and
never changes an existing summary.

To retain a summary for review and gate preparation without copying stdout,
create its ignored parent and publish it directly:

```bash
mkdir -p .local-exclude/coordination/<slice>/validation
bash tools/validation/bounded-run.sh \
  --summary-out .local-exclude/coordination/<slice>/validation/workspace-tests.json \
  --reusable-local \
  workspace-tests -- cargo test --workspace --all-targets
```

The output file and stdout line are byte-identical. Publication is atomic and
create-only: a missing parent or existing target stops before the validation
command, and a concurrent target collision is never overwritten. Add the
published file to the immutable review packet so its manifest supplies the
path and hash to `slice gate prepare`. This stores new evidence only; it does
not reuse an earlier result. A reuse decision belongs to the later reviewed
Slice gate request, not this runner.

The wrapper changes presentation, not validation semantics. Its logs are
temporary operational artifacts: keep a required failure log only while the
finding is unresolved and discard completed logs with the Slice worktree.

## Consolidate one candidate gate

Use this section only for a formal Slice. Ordinary work reports its checks
and limits directly; it does not need a gate request or immutable evidence chain.

`cargo xtask slice status <slice>` reuses a current candidate's gate request
even when its review was performed directly without a packet. When gate
evaluation is next and one matching request exists, status returns `run_gate`,
its `next_argv`, and the Slice's
`next_working_directory`; run that argv in the returned directory. Multiple
matching requests require explicit selection. Discovery does not validate the
request or approve the candidate: the gate rechecks the evidence and returns
the actual next action. Dirty candidates and broken published review ancestry
remain blocking conditions.

Once a Slice candidate is a clean commit, save each bounded validation JSON
summary and each final review response as a separate local file. Record their
exact hashes, the candidate commit, canonical diff hash, required lenses,
known unverified environments, risk classification, and human-origin approval
in a `yo.slice-gate-request/v1alpha1` request. Then run:

```bash
cargo xtask slice gate /tmp/<slice>-gate.json
```

A minimal request with one declared check and one completed lens has this shape
(repeat the evidence entries when more checks or lenses apply):

```json
{
  "schema": "yo.slice-gate-request/v1alpha1",
  "candidate_commit": "<full-commit>",
  "required_lenses": ["fresh-context"],
  "validation_evidence": [{
    "name": "workspace-tests",
    "argv": ["cargo", "test", "--workspace", "--all-targets"],
    "result_path": "/tmp/workspace-tests.json",
    "result_hash": "sha256:<summary-hash>",
    "candidate_commit": "<full-commit>",
    "reused": false
  }],
  "review_evidence": [{
    "lens": "fresh-context",
    "reviewer": "provider/session",
    "route": "model-high/provider/model/session",
    "verdict": "clear",
    "candidate_commit": "<full-commit>",
    "diff_hash": "sha256:<canonical-diff-hash>",
    "result_path": "/tmp/fresh-context.txt",
    "result_hash": "sha256:<response-hash>"
  }],
  "known_unverified_environments": [],
  "risk": {
    "classification": "human-attention",
    "rationale": "changes workflow authority"
  },
  "approval": null
}
```

After exact human approval, replace `null` with `kind: "exact_candidate"`, a
`human/<identity>` authority and scope, plus the same `candidate_commit` and
`diff_hash`. A routine request may instead use `kind: "standing_routine"` and
omit those two exact identity fields only when its human-origin scope covers
the work and no unverified environment remains.

The command checks the bound Slice scope, clean `HEAD`, path-derived minimum
lenses, evidence file hashes, candidate/diff identities, review routes, and
approval shape. It returns a single `yo.slice-gate-result/v1alpha1` JSON line
with exactly one `next_action`: `validate`, `review`, `approve`, or `integrate`.
It never runs those actions itself. A changed candidate, stale diff, mutated
evidence file, or omitted path-derived lens fails closed rather than producing
a next action.

This is an evidence-consistency check, not proof that the declarations are
true. The coordinator still owns the completeness of the validation plan,
semantic review lenses, risk classification, and recorded verdict. Keep the
request and evidence in ignored coordination storage or outside the worktree,
then remove them when the Slice closes.

## Slice-close baseline

This section is for an explicitly selected formal Slice. Ordinary completion
requires affected package and consumer checks, documentation checks when
relevant, and `git diff --check`. Run the full workspace suite for shared
runtime/build changes, releases, or unresolved cross-package impact. Do not
run it for unrelated documentation or local implementation changes.

For a formal Slice, after focused checks pass, run the repository baseline:

```bash
bash tools/validation/bounded-run.sh workspace-tests -- cargo test --workspace --all-targets
bash tools/validation/bounded-run.sh workspace-clippy -- cargo clippy --workspace --all-targets -- -D warnings
bash tools/validation/bounded-run.sh hk-candidate -- \
  hk check --check --from-ref BASE_SHA --to-ref CANDIDATE_SHA
```

`cargo test` runs the normal test set and compiles ignored tests; it does not
execute ignored environment tests. `hk check` selects repository checks from
`hk.pkl` according to the changed paths, including formatting, test
explanations, affected crate checks, Methexis checks, and Developer Docs checks.
Installation and hook usage belong to
[`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#local-checks).

For a staged Methexis change containing only `methexis/sources/` and
`methexis/knowledge/` paths, the hook first requires the working Methexis tree
to match the index exactly and contain no untracked Methexis paths, then runs
only the `records` and `relations` classes. This admits a semantic-first
candidate while its prior Projection is intentionally stale. Any staged
Projection, approval, Checkpoint, active record, or other Methexis path keeps
the complete authority-aware validation path.

Use focused checks from the local Slice contract while editing, then run this
Slice-close baseline once the outcome is complete. For the exact staged
Methexis activation interval, `hk` uses prospective validation and defers the
ordinary Methexis tests; immediately after integration, run the ordinary full
Methexis check and tests against trusted `develop`.

Prepare that activation worktree from clean `develop` with
`cargo xtask slice create-activation <request.json>`. The generated contract
leases the active record, the Checkpoint tree, and the two registered context
manifests. Its focused `methexis check --staged-activation` admits exactly one
new immutable Checkpoint. Slice creation is coordination setup, not evidence
that the prospective transition is valid.

For a later independent activation review, use the explicit v1alpha3 review
request only after the enabling workflow implementation is already trusted.
The focused tests above prove the trusted-capability bootstrap, exact
activation-only path boundary, proposal identity, and canonical packet replay;
the candidate still requires staged activation validation before integration
and ordinary full Methexis validation immediately afterward.

If the Slice changes a platform or external-environment boundary, add the
relevant matrix command rather than claiming the baseline covered it.

Do not rerun the unchanged baseline merely because a reviewed candidate was
squashed. New fast acceptance may use a candidate-bound
`hk check --check --from-ref BASE_SHA --to-ref CANDIDATE_SHA` result in place of
duplicate commit hooks only when the integration HEAD is the candidate base,
those arguments equal the gate's exact base and candidate, and the result is
passing, non-reused, no-external-state evidence whose OS,
architecture, and Rust/Cargo fingerprint still match. It records that choice as
`candidate_hk_receipt`; otherwise it records `git_hooks` and runs the hooks.
This reuse never replaces validation or review of the candidate itself.

The Slice-close cleanup command is not part of this validation baseline. After
the gate returns `integrate`, `slice commit prepare` appends its exact review
trailers to a human-authored semantic message without staging or committing.
New `slice accept prepare` uses
`yo.slice-accept-prepare-request/v1alpha3`. It needs only the ready gate and
human-written message source; `push_remote` is optional. It derives a compact
`yo.slice-close-prepare-request/v1alpha2` and the corresponding candidate,
validation, and review-evidence count instead of
blocking cleanup on manually reconstructed lane, packet, and timing totals.
The compact path requires no known unverified environment because it cannot
derive that environment's missing command; use the frozen observed-metrics path
when the gate must retain such a mapping.
Frozen earlier requests keep their original observed-metrics shape. Close
preparation publishes the standard `close-metrics.json`; it does not itself
plan or apply cleanup.

The close plan
publishes its plan directly to the requested file, then consumes the already
accepted result afterward and rechecks the exact refs, review trailers, patch
identity, worktree cleanliness, binding, contract hash, and plan hash before
removing the local worktree, standard transient Slice contract, and Slice
branch. Directly writing the complete metrics file remains supported. The plan binds that
record to the exact Slice candidate and accepted commit; apply rejects changed
metrics. The plan also lists every immediate coordination entry it will
retain, including the metrics; apply rejects a changed list and never removes
those entries. Store the plan outside both the removed worktree and that
Slice's coordination directory. See the integration workflow in
[`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#review-and-integration).

## Useful owners

- Hook selection: [`hk.pkl`](https://github.com/Yon-Fandorin/yo/blob/develop/hk.pkl)
- Structured repository checks: [`tools/xtask`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/xtask/src/lib.rs)
- Unix host compile check: [`tools/validation/yo-cli-unix-matrix.sh`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/validation/yo-cli-unix-matrix.sh)
- Rendering parity fixture: [`crates/yo-tui/tests/fixtures/rendering-parity/README.md`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/tests/fixtures/rendering-parity/README.md)
- Test explanation policy: [`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#test-code)


### Rich content preview

Inside `/preview`, `showcase` combines `syntax`, `charts`, `images`, and
`media-errors`. `image /absolute/path.png` reads only the explicitly named regular
local file (up to 1 MiB) into this offline preview; it never modifies it. Normal
Markdown layout does not fetch paths or URLs. Embedded PNG/JPEG data URIs are
decoded with a 1 MiB encoded-file limit, 2048-pixel dimension limits, and a
32 MiB decoder allocation budget. PNG/JPEG EXIF rotations and reflections are applied before
thumbnail generation and native PNG conversion; displayed dimensions follow the oriented
image. Small thumbnails never upscale beyond source dimensions. Image color-depth evidence is
independent of semantic code colors: overriding code text to terminal/default or RGB
preserves the image cells and native raster eligibility. Mono/unknown-color fallback
remains explicit. The Rust preview accepts `--terminal-code-text` to check this with
`images` or `image-orientation`. The `image-orientation`
preview uses a sideways-stored JPEG that must appear upright. Four bounded thumbnails and their original-resolution PNG payloads are cached for reflow.
Cell images use at most 64 columns and 20 rows; monochrome/ASCII uses density
characters. Fullscreen can transmit PNG at original resolution with Kitty graphics;
JPEG and images requiring EXIF correction are converted, while unchanged PNG bytes
are preserved. A fully visible, uncovered image gets a native
placement; clipping/overlay coverage falls back to cells. Inline remains cell-only.
Known direct Kitty-compatible terminals use native transport automatically;
`YO_TUI_IMAGE_PROTOCOL=cells` disables it. Unknown terminals and tmux default to
cells. Explicit `kitty-tmux` selects DCS passthrough only after the operator has
confirmed outer support and `allow-passthrough on`. No iTerm2/Sixel encoder or model
image-input support is included. Native protocol-byte tests do not establish
physical terminal rendering compatibility.

Language fences use Syntect's bundled grammars. Unknown languages, blocks over
64 KiB, and lines over 4096 bytes retain literal code without syntax colors.
`chart` fences accept up to 64 nonempty `label: finite-number` rows; `sparkline`
fences accept up to 64 space-separated finite numbers. Invalid data retains its
source. Chart values are always shown, and negative bars share the same zero axis.
Each validated sample retains its input numeric token separately from the finite
value used for geometry. Bar labels preserve exponent notation, explicit plus signs,
negative zero and trailing decimal places at both stacked and aligned widths.
Verify full-source/plain-output preservation separately from the display projection.


The component owners are `transcript/layout/markdown/code.rs` (syntax roles),
`chart.rs` (validated series and responsive plots, using meter for bars), and
`image.rs` (bounded decoding/cache and cell fallback). `surface/raster.rs` owns
protocol-neutral PNG placement metadata. `terminal/graphics.rs` owns Kitty
chunking, scoped image IDs, passthrough, and deletion. Fullscreen frame commit
still follows successful output; covered image metadata is invalidated by cell
writes. Check resize, stale placements, failed frame retries, and cleanup in a
real supported terminal before claiming native compatibility.

`stepchart` accepts the same finite-number series and holds each value horizontally
until the next sample, then changes vertically (horizontal-then-vertical steps).
Samples are equally spaced indices, not inferred timestamps. This follows the `hv`
line shape illustrated in [Plotly's line-chart documentation](https://plotly.com/javascript/line-charts/).
It shares chart roles, value/sample axes, the 64-sample bound, source fallback and
narrow sparkline representation. `charts` includes a concurrent-task example.

`linechart` adds connected Braille plots with value/sample axes; narrow widths
fall back to sparklines. Bar rows use aligned label/value columns when space
permits and stack labels at narrow widths. Signed values share a zero axis.
Trend normalization retains differences between zero and subnormal finite values
and avoids overflow across opposite large magnitudes. Line plots reserve the actual
scientific-notation axis-label width so all six axis rows stay aligned. The last-value
legend retains the input numeric token instead of expanding exponent notation into
hundreds of decimal places. The `charts` preview includes both numeric boundaries.


### Configurable output and source comparison

`OutputPreferences` is the public session-owned layout input. The CLI maps
`tui.max_body_width` (positive u16), `tool_head_rows`, `shell_tail_rows`, and `diff_head_rows` (u16)
from config.yaml to `TuiSession::with_output_preferences`. The appearance snapshot
owns the resolved values; components receive layout settings, not CLI/config paths.
A requested one-column body uses two columns to retain wide Unicode. Default head counts remain 2/6. Folded activities keep the configured opening rows
and three trailing rows; folding begins after the head count plus six rows and
only when the wrapped hint saves space. Zero head rows is allowed. Saturating
arithmetic keeps the maximum u16 value valid. `/changes` bypasses folding.
Tests in `runner/tests/appearance.rs` verify consumed frame output, independent
tool/diff settings, wrapping, retained source, and preservation across theme choice.
CLI config tests reject zero width, negative/overflow counts and unknown keys.

`show_images` defaults to true; false skips decoding and leaves alt text with a
visibility notice. `image_max_width` is a positive u16, defaults to 64, and clamps
larger values to 64. Both use `OutputPreferences` builders and survive theme
changes. Frame tests verify 1/8/64-column placements, hidden-image raster removal,
original PNG byte retention and unchanged plain output. Invalid image payloads
are not decoded when hidden; invalid booleans and zero/overflow widths are rejected.


Reference inspection used pi commit
`7d8ab31a477ecc07b36f56ffcae58c79307a68be` and Codex commit
`e1eb98461cd42730e7b1a890c3100303d7091bc2`:
[pi output components](https://github.com/earendil-works/pi/tree/7d8ab31a477ecc07b36f56ffcae58c79307a68be/packages/coding-agent/src/modes/interactive/components),
[Codex history cells](https://github.com/openai/codex/tree/e1eb98461cd42730e7b1a890c3100303d7091bc2/codex-rs/tui/src/history_cell).
This comparison is not feature-parity evidence. Additional tool-specific content styles,
animated-status palette roles, reasoning visibility, and several host observations
still require implementation and validation.

Codex webSearch items retain search queries, open-page URLs and find patterns
through ToolCall snapshots carrying ToolOutput. Original query/action JSON remains
available to custom renderers. Optional non-null `results` now survives in the
result object and plain export, including explicit empty arrays and unknown result
shapes/fields. Absent/null results do not fabricate a result list. The app-server
schema intentionally leaves entries opaque, so default result presentation is
literal JSON; custom renderers receive the full value. Item-level query supplies
the fallback when action detail is absent or empty. Unknown or malformed action
fields remain readable JSON. Existing body width, folding and code colors apply;
no URL or image is fetched. Typed callbacks, 80/24/80 frames, original exports and
wire start/completion are tested. Structured citation interpretation remains open.

Public `item/reasoning/summaryTextDelta`
notifications now update the existing ModelWork activity before completion. Sparse
summary indices accumulate independently and publish ordered snapshots; initial
summary parts are retained, and completion replaces the displayed summary.
Thread/turn/item identity and reasoning-item ownership are validated before mutation.
Raw `item/reasoning/textDelta` remains outside this display path. Reasoning completion retains only the `summary`
array; an absent public summary never falls back to raw reasoning `content`.
`runtime::tests::coding_events` checks these wire-to-semantic payloads.
`search` and `reasoning` preview their presentation without model calls.


`ThemeOverrides` supplies semantic `ThemeRole` colors independently of the built-in
palette. `ThemeColor` accepts RGB or terminal inheritance; CLI `tui.colors` uses
exact snake-case role names with quoted `#RRGGBB`/`terminal` values. The appearance
snapshot retains the selected base theme and overrides; changing themes reapplies
overrides, clearing overrides restores the base palette. RGB is mapped to the
nearest deterministic 256-color cube/grayscale entry for limited terminals; unknown
color capability and Mono resolve to terminal defaults. Styles retain their
attributes and layout. `runner/tests/appearance.rs` verifies real user/code/diff
cells across TrueColor/Limited/Unknown and theme roundtrips; config tests reject
invalid roles and malformed or non-ASCII hex strings before terminal startup.
The offline Rust example's `--custom-colors` option exercises this public API.
These slots do not yet replace the animated activity gradient or add a per-tool
renderer extension. Reference: pi's pinned `theme/theme.ts` separates semantic
foreground/background roles from capability-dependent color encoding.

Completed ModelWork summaries replace the owned progress heading with an explicit
completed/interrupted/failed label and request redraw. Literal matching words in
the payload remain intact; Plan and usage headings keep their own presentation.
The activity projection and inline publication tests cover this terminal transition.


MCP result content and dynamic `inputText` blocks now use literal multiline text.
Text resources expose URI/body/metadata; resource links expose their identity and
URI without fetching. `structuredContent`, extension fields and unsupported or
malformed blocks remain visible through JSON fallback. `coding_events` tests
assert actual snapshots for mixed content, dynamic results, empty content and
unknown shapes. `/preview mcp` exercises the existing width/folding styles with
resource output. MCP and dynamic embedded PNG/JPEG blocks now use the explicit ToolOutput profile
and native/cell image path described below. Remote URLs are not fetched.


`ToolRenderer`/`ToolRenderInput` expose a session-local tool-body Markdown callback
through `TuiSession::with_tool_renderer`. Input `expanded` reports the session activity
expansion request; a renderer can return a compact summary or a richer expanded body.
Ctrl+O changes the layout configuration and invalidates cached results, including
when the callback handle is unchanged. Ordinary host folding still applies to the
returned body. The standalone `--custom-tools` example uses this for docs.search.
Input `outcome: Option<TranscriptActivityOutcome>` carries the observed terminal
Completed/Failed/Interrupted state; None means no terminal outcome has been observed,
not proof that physical execution has started. Payload words and structured error
fields do not establish this state. Outcome changes invalidate the item layout even
when source text is unchanged. The `mcp-failure` fixture exercises partial results and
failed status, with a custom collapsed summary in the standalone example.
The existing activity presentation owner
retains ToolCall/ToolResult kind; the layout owner invokes the callback only for
those bodies, keeping status headings and outcome footers outside it. Plain output
bypasses the callback. `None` or unrenderable content falls back to original text.
Callbacks must be pure and deterministic; handles compare by shared allocation
identity, so replacement invalidates layout caches and cloned handles keep identity.
Theme changes and output preferences retain the handle. Image display/width limits
apply to custom Markdown; folding removes overlapping native rasters and shifts
fully retained ones with their fallback cells. Frame tests cover code/table/PNG,
failure/source retention, hide/width settings, fallback/handle reset, non-tool exclusion
and folded raster positioning. The Rust offline example supports `--custom-tools`.
The callback also receives `output: Option<&ToolOutput>` with admitted structured data.

`yo_core::ToolOutput` owns the exact `yo.tool-output/v1` presentation profile.
`to_snapshot`/`from_snapshot` bound encoded data to 16 MiB, require a nonempty tool
identity, reject unknown envelope/output fields and exact-schema mismatches, and
preserve arbitrary original arguments/result/contentItems/error JSON. Codex MCP
and dynamic tool snapshots now use this profile; their plain projection summarizes
embedded media without dumping base64. This changes no journal enum, discriminator
or record field: the profile remains ordinary bounded text stored by existing
message segments/seals. Durable tests reconstruct a multi-segment profile and verify
identical JSON/media bytes; admission tests cover exact capacity and the first excess byte.

Only ToolCall/ToolResult presentation admits this profile. Default output uses JSON
argument panels, fenced literal text, resource bodies and explicit PNG/JPEG images.
Literal tool fences cannot escape into Markdown media. Dynamic data-image URLs use
the same decoder; remote URLs and audio receive text placeholders. Original JSON
remains available to callbacks and in the journal. Unknown schema/malformed data
stays literal. Plain export uses `plain_text`; retained records keep the full profile.
Frame tests cover default MCP and dynamic images, original PNG bytes, image-off/width,
plain export and typed custom renderer input. `mcp-image` is an offline snapshot
fixture using an encoded 128×64 PNG; no model call or image input is implied.


`ActivityNotice` owns the exact `yo.activity-notice/v1` ModelWork presentation
profile with title, literal message and info/warning level. It uses the same bounded
text snapshot carrier as tool output without changing journal record variants.
Only typed ModelWork presentation admits notices; ordinary ModelWork styling and
non-folding behavior remain. Notice titles use the semantic warning/heading palette,
and delivered notices do not become “Model work completed”. Plain export is readable.
Tests cover exact schema/level/field admission and final warning-color frame output.
Interrupted/failed notice delivery keeps its owned footer outside the profile; empty
deltas do not erase notice identity.

Codex `error` with `willRetry: true` now emits a notice before the turn finishes and
clears any previously stored error for that turn. It does not terminate the turn or
initiate another model call. Subsequent final failures use terminal error evidence;
late retry notices for completed turns are ignored. Missing willRetry retains the
legacy non-retry interpretation; non-boolean values fail protocol validation.
Backend tests cover successful/failed terminal outcomes and late-event exclusion.
`retry` is the offline preview scenario. Retry counts/delays are not inferred.
Session-level `warning`/`configWarning`, `deprecationNotice`, and `guardianWarning`
notifications use the existing observer
(`CodexWarning`, with the source-compatible `CodexCompatibilityWarning` alias).
Global warnings are observed during calls; scoped warnings wait for validated thread
binding and other-thread warnings are excluded during polling. Display bodies are
bounded to 8 KiB, control characters escaped, and one client poll consumes at most
32 warning notifications. The CLI collector deduplicates and retains at most 32
distinct warnings plus one suppression notice. Live TUI delivery wakes the idle
frontend through `AgentPoll::Notice`, preserves semantic titles/colors and shares
one publication cursor with stderr output. Session notices are ephemeral transcript
items, without fabricated Turn identities or journal records. Print mode retains
stderr diagnostics. Tests cover thread isolation, bounds, idle wakeup, single
publication, and the actual styled frame before any turn. `/preview warning`
exercises the same session-notice presentation without a model call. Within the preview,
`deprecation` shows migration guidance and `approval-warning` shows an approval review
warning. These fixtures start no Turn and change no permissions.
Warnings observed during a frontend poll are delivered before that poll’s original
result, including closure and failure. The Session retains the result across frontend
reentry; draining notices does not consume another agent result. Deprecation guidance
and thread-scoped approval warnings preserve their distinct titles and use the existing
warning/body palette. These notification fields follow the [Codex protocol source](https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/src/protocol/v2/notification.rs).


Codex `contextCompaction` items map to one ModelWork activity with an info-level
`ActivityNotice`. The item has no status, summary or token-count fields in the
observed app-server schema: `item/started` selects “Compacting context” and
`item/completed` replaces it with “Context compacted”. No metrics or summary are
inferred. The existing real thread/turn/item binding and interruption cleanup apply;
late completion after interruption stays suppressed. The heading uses configurable
`accent` color. Backend tests cover progress/completion ordering and interruption;
the TUI frame test covers in-place replacement, color, source export and absence of
raw profile JSON. `/preview compaction` shows the transition without model calls.
This is host-observed automatic compaction, not support for the delegated `/compact`
command. A separate explicit summary profile handles supplied text:
pi supplies a summary and pre-compaction count, while this Codex item does not.


`ActivitySummary` is the bounded `yo.activity-summary/v1` ModelWork text profile,
with `SummaryKind::Compaction`, `Branch` or `Reasoning`, complete Markdown `summary`, and optional
observed `tokens_before` for compaction only. Unknown kinds/fields, negative counts,
branch/reasoning counts and the first byte above 16 MiB are rejected. This is presentation;
it neither executes compaction nor creates a branch or changes journal records.
The TUI owns a semantic accent heading, preserves failure/interruption footers and
empty-delta identity, and displays an explicit empty-summary fallback. Summary
Markdown uses existing code/table/chart rendering without attachment authority.
`tool_head_rows` also controls folded summary rows; Ctrl+O expands both tools and
summaries, preserving source export. Body width and the existing palette apply.
Tests exercise profile boundaries, both kinds, folding/expansion, failure footer,
custom accent, source preservation, and narrow/empty/image-fallback frames.
`/preview summary` and `/preview branch` provide full-snapshot offline examples.
No live Codex summary is inferred from its body-less contextCompaction item.


Codex public reasoning start/summaryTextDelta/completion now emit the explicit
Reasoning profile. Sparse public-summary parts retain their order and the completed
summary replaces streaming text. Empty public summaries remain explicit; raw
reasoning content/textDelta is never a fallback. `tui.colors.reasoning_text` and `ThemeRole::ReasoningText` independently color public
reasoning prose and the hidden-summary hint. The default follows `muted`; an explicit
reasoning override takes precedence. Compaction summaries, code/syntax colors, status
footers and plain export retain their existing roles. Frame tests cover independent
colors, visible/hidden states and 80/24/80 resizing; CLI parsing accepts the new role.
`tui.show_reasoning` defaults true;
`OutputPreferences::with_reasoning(false)` displays a short hidden-body placeholder
while preserving owned failure footers, all other activity families, records and
plain export. Long public summaries and provider reasoning also obey activity folding:
the opening rows, tail and failure footer stay visible; Ctrl+O restores the complete
body after resize, and folded hints retain the reasoning color. Changing preferences invalidates layout and restores the same content.
Tests verify the typed backend handoff, streamed ordering, private-content exclusion,
CLI boolean admission and actual hide/show/hide frames with warning/plan/compaction
controls. `chat_preview --hide-reasoning` exercises this preference offline.


Mermaid fences route through `markdown/diagram.rs` and pinned
[`mermaid-text` 0.57.0](https://docs.rs/mermaid-text/0.57.0/mermaid_text/).
The source-to-text call uses natural layout, then measures every output row with
Yo's cell-width rules; diagrams wider than the available prefixed/padded body keep
the original code. This avoids the dependency's documented narrow-label overlap.
Code palette/background, body width and plain export remain owned by Yo.
`tui.show_diagrams` defaults true; `OutputPreferences::with_diagrams(false)` keeps
fenced source. Public reasoning summaries keep Mermaid source even when diagrams
are enabled. No external renderer process, browser or image protocol is invoked.
Input work is bounded before rendering: 8 KiB, 64 newline/semicolon segments,
512 whitespace tokens and 512 bytes per source line. Output above 256 rows or
64 KiB falls back; control/unrenderable characters also preserve source. Gantt
uses source view because the dependency can derive missing dates from the clock;
style/classDef/class/click/linkStyle directives use source view rather than being
silently discarded. Other parsing failures and width limits show a short reason
above the retained code. Flowchart and sequence paths, source fallback, first excess
statement, palette, display preference and cache invalidation have tests.
`/preview diagrams` is the real Rust offline example. Other Mermaid families depend
on the pinned library and are not claimed to have complete parity with Mermaid.js.


`ActivityPlan` is the exact bounded `yo.activity-plan/v1` ModelWork text profile,
with optional literal explanation and ordered `PlanStep { text, status }` values.
`PlanStepStatus` admits pending/in_progress/completed only. Codex turn/plan/updated
maps wire statuses explicitly, rejects malformed explanations, reuses one activity
per turn and ignores late plans after turn completion. The journal shape stays
unchanged. TUI rendering uses success/accent/muted for completed/active/pending
steps, bold active steps, reported completion counts and hanging indentation when
the body has at least six columns. Smaller widths use full literal wrapping.
Explanations and steps never become Markdown or attachments. Empty plans show
“No steps provided.” instead of a fabricated ratio. Activity completion/failure
preserves the reported step states; owned failure footers remain visible.
Tests cover profile admission, typed backend update ordering, actual customized
frame colors, narrow wrapping, empty plans, literal source export and outcome
preservation. `/preview plan` now supplies the same structured output profile.
The separate proposed-plan Markdown stream uses the document profile below.


`ActivityDocument { title, markdown }` is the exact `yo.activity-document/v1`
ModelWork text profile, with a nonempty literal title and bounded 16 MiB snapshot.
It reuses existing journal records and the summary Markdown presentation path,
without misclassifying proposed plans as compaction or reasoning summaries.
Documents share body width, semantic accent/code styles, diagram preferences and
Ctrl+O folding; they neither authorize actions nor grant image attachment authority.
Codex plan items retain their start text in the item binding, concatenate validated
`item/plan/delta` fragments into complete document snapshots and replace them with
`item/completed` text. Invalid target kinds/threads and oversized start/delta/final
documents fail protocol validation; ordinary assistant deltas keep their prior path.
The final source is re-rendered at each width instead of storing display fragments.
Tests cover typed backend sequence, target rejection, profile identity, narrow/wide
frame reflow, final-over-draft replacement, interrupted footer and unchanged plain
export while reasoning is hidden. `/preview proposed-plan` shows a timed draft/final
transition in the real offline Rust agent. Empty documents have a text fallback.

Codex `item/commandExecution/terminalInteraction` notifications become independent
activity documents: empty stdin reports a background-terminal wait; nonempty stdin
reports input sent, with process ID and the known command. Command identity survives
item completion until its turn ends; notifications after turn completion are ignored.
Unknown commands, cross-thread/turn targets and non-string input fail validation.
A fence longer than every input backtick run preserves literal Markdown and control
characters, without appending input to command stdout or sending new process input.
Existing code colors, body width and folding apply. Backend tests cover target
validation and lifecycle; frame tests cover narrow/wide reflow, literal image syntax,
control notation and stable export. `/preview terminal-wait` and
`/preview terminal-input` use offline Rust fixtures without process interaction.

Turn completion uses optional Codex `durationMs` (nonnegative int64) for an
`ActivityNotice` containing the actual completed/failed/interrupted status, formatted
elapsed time and exact milliseconds with provider attribution. Missing/null duration
emits no timing notice; zero remains a reported zero. Invalid values fail before
terminal state mutation. Plan/item interruption cleanup precedes the notice and the
turn terminal event follows it; repeated turn completion emits neither twice. Existing
notice accent/warning colors and width reflow apply. Tests cover all three outcomes,
absent/null/zero values, precision, first int64 excess, ordering, duplicate suppression
and narrow/wide frame export. `/preview turn-duration` is an offline fixture. This
does not infer API/tool runtime metrics or wall-clock timestamps.

The default structured tool renderer infers source language for exact `read`,
`read_file` and `readFile` identities from string `file_path`/
`path` arguments; the path is included in the tool heading. Embedded text resources
use their URI suffix and separate identity from source code. Recognized programming
file extensions use existing Syntect semantic colors/code backgrounds; unknown and
Markdown/diagram extensions remain literal text. Error outputs (`error` or result
`isError: true`) are never treated as successful source. This is presentation-only
suffix inference, with no path resolution or file/network reads. Original arguments,
result metadata and plain export remain; a custom `ToolRenderer` takes precedence.
Frame tests verify custom keyword colors, narrow/wide source wrapping, literal fences
and image syntax, error/unrecognized-tool fallback, and callback override.
`/preview file-read` supplies an offline structured Rust result. The managed backend now supplies the same profile after semantic admission;
write proposals and batch-read panels use the presentation paths described below;
edit proposals and native mutation summaries are also described below.


Managed tool completion emits `ToolOutput` using the admitted function-call arguments
already retained in the current turn replay and the admitted, bounded output string.
Execution-only raw arguments never enter this projection. Result content is one
literal text block, including output that happens to resemble JSON or media; metadata
retains call/tool/host identity and completed/failed/interrupted outcome. Model replay
and the next connector request retain the exact admitted output string. For results without separate retained output, if expanded
presentation exceeds the 16 MiB profile limit, the existing call-id/output receipt
remains the fallback rather than failing an already performed action. Startup result
metadata, approval and execution scheduling are unchanged. Tests cover redacted
arguments/output in typed presentation, replay and subsequent request, all three
execution outcomes, literal JSON-shaped media, and a 2 MiB control-character result
whose expanded presentation exceeds the profile limit while replay remains accepted.
Well-typed execution metadata is grouped into one **Execution details** text block after
the command and result, with the existing truncation warning retained. Unknown or malformed
fields keep their generic display. The 20/40-column surface tests also check original
snapshot, plain export and custom-renderer input preservation.
Source colors and custom renderers
are verified by the structured TUI frame tests; no live provider call is implied.

Exact `write`/`write_file`/`writeFile` tools display string `content` arguments as
**Proposed file content**, using path-based syntax colors. Only the displayed
arguments JSON omits that duplicate content field; the original typed arguments and
plain export remain unchanged. Empty content has an explicit empty-file hint; invalid
content remains visible in arguments. Results and errors remain separate literal
output, including after a failed write. Existing body width, code palette, folding
and custom renderer precedence apply. No renderer writes or previews filesystem diffs.
Managed FunctionCallDone now publishes admitted arguments through ToolOutput before
execution/approval scheduling, with the prior receipt as oversize fallback. A completed
ToolCall with structured arguments and no result/content/error is titled **Tool call
prepared** in both screen and export; it does not claim successful execution. Tests
cover pre-result and failed proposals, empty/invalid content, narrow reflow, custom
keyword colors and callback inputs, preparation heading and admitted-only backend
projection. `/preview file-write` is an offline example that writes no file.

The exact `read_files` tool can render its native text result as ordered file panels.
A complete `{"results":[...]}` object supplies per-file path/status, successful
start/end/total/content and optional next_offset, or an error string. Panels show
reported line ranges, source-language code, the next reading position, empty files
and individual failures. Content line count and range/continuation consistency are
validated; unknown fields/statuses, malformed/truncated JSON, more than 8 items,
empty result arrays or input above 256 KiB fall back to the entire literal source.
This parser uses the workspace's existing serde_json dependency and performs no I/O.
The typed result and plain export remain intact, and custom ToolRenderer overrides
the default. Frame tests verify ordering, custom syntax colors, 80/24/80 reflow,
empty/error states, malformed/extended input, 8/9 item and exact/first-excess byte
limits, source retention and custom callback inputs. `/preview files-read` is an
offline fixture with a partial Rust file, an unavailable file and an empty file.

Exact `edit`/`edit_file`/`editFile` tools display valid oldText/newText pairs as
**Proposed replacements**, accepting an edits array or a single pair. Displayed
arguments omit only the recognized replacement fields; original typed arguments and
export remain available. Diff blocks use existing addition/removal colors and tool
folding, identify each replacement, preserve deletions and explicitly identify missing
trailing newlines in old/new text. They supply no invented file positions or claim
that proposals were applied. Empty old text, unknown edit fields, ambiguous shapes,
more than 256 pairs or combined old/new text above 256 KiB retain literal arguments.
Successful native edit_file/write_file text receipts with exact path/status/metric
fields show reported replacement counts/written bytes; malformed, extended or failed
results remain literal. Parsing is bounded to 16 KiB and performs no filesystem I/O.
Frame tests cover custom diff colors, narrow reflow, failures, deletion/newline
semantics, raw callback arguments, 256/257 pairs and exact/first-excess text bytes,
and successful/invalid mutation metrics. `/preview file-edit` shows offline proposed
replacements and explicitly states that nothing was applied. Full observed file
diffs remain the responsibility of file-change events and host-provided evidence.

Exact `run_command`/`bash`/`powershell` identities display string command arguments
in a separate code panel, omitting the duplicate field only from displayed JSON.
Remaining arguments (such as timeout), original profile, export and custom callback
inputs stay intact; empty commands have an explicit hint and invalid values remain
JSON. Bash-compatible commands use existing syntax colors; unsupported language
highlighting retains literal code. Complete native run_command receipts with canonical
integer or signal status and one unambiguous stderr delimiter show exit status and
literal stdout/stderr sections, including failed commands and empty-stream hints.
Any truncation marker, invalid header or repeated delimiter preserves the combined
source; no timing, channel boundaries or process status are invented. Code fences
prevent output Markdown/media from being interpreted. Frame tests verify custom bash
keyword colors, 80/24/80 reflow, status/signal/empty/error cases, ambiguous/truncated
fallback, control notation, literal image syntax and exact callback inputs.
`/preview shell` uses an offline fixture without starting a process. Separate raw
stream provenance and provider full-output-file metadata remain future extensions.

Managed tool completion now retains a boolean `result.truncated` observation from
the execution host or local output-byte truncation. Semantic output admission and
model replay strings remain unchanged. The native `list_files` renderer requires
that boolean and a successful result; it displays ordered literal file/directory
rows, counts of returned entries/directories, empty-return hints and an explicit
partial-listing notice. A trailing slash denotes a directory under this native
contract. The truncation marker is removed only when truncation was reported; an
identically named file in a complete listing remains visible. Missing metadata,
control characters, incomplete lines, paths above 1024 bytes, more than 1024 entries
or source above 256 KiB retain the full literal result. No directory is opened and
no tree hierarchy or total directory size is inferred. Code background, width,
folding and custom renderer inputs remain configurable. Backend tests cover host
truncation and exact/first-excess local output limits; frame tests cover narrow/wide
reflow, empty/partial/unknown states, literal names, source/callback retention and
exact/first-excess entry/path/input limits. `/preview files-list` is an offline
partial directory fixture. Native execution still permits larger listings; these
presentation limits select a literal fallback without dropping execution output.

Embedded `resource` blocks retain outer annotations/unknown fields and inner resource
metadata alongside URI, MIME type and body. Exactly one typed text/blob body is required;
malformed or ambiguous payloads remain whole literal JSON. Successful text preserves URI
suffix highlighting. PNG/JPEG blobs use the existing image decoder, visibility/width settings
and renderer; other binary types retain a placeholder without decoding or fetching.
Tests cover inner/outer metadata, 80/24/80 reflow, malformed payloads, literal text, source and
custom callback retention. The existing structured-image test now also verifies embedded PNG
bytes and configured width with images enabled/disabled. `/preview embedded-resource` is offline.

`tui.colors.chart` / `ThemeRole::Chart` independently colors chart traces, bars and axes.
Without an override it inherits the accent, including explicit accent overrides. Changing or
removing the chart color preserves heading styles and source; frame tests cover
80/24/80 line/bar reflow and override/reset. `--custom-colors` in the Rust preview shows amber
charts with purple headings.

Named `linechart`, `stepchart` and `scatterchart` data uses one `name: values` line per
series, after optional `height: N`. One to four unique names (1–64 UTF-8 bytes, no controls)
and 1–64 finite samples per series are accepted. Line/step series require equal sample counts;
scatter series may differ. All series share X/Y ranges; line/step X is the zero-based sample
index. Numbered point markers match the legend, and `×` identifies overlap at plot-pixel resolution
without claiming an exact mathematical intersection. Different pixels in a shared cell retain
their combined Braille shape in a neutral color. Colors use `chart`, `chart_2`,
`chart_3`, `chart_4` / `ThemeRole::Chart` through `Chart4`. Monochrome retains point numbers;
ASCII maps Braille to `*` and overlap markers to `x`. At narrow widths the legend, ranges and
original named values remain instead of a clipped plot. Duplicate/oversized names, excess
series/samples, nonfinite numbers or unequal line/step lengths fall back to complete source.
Tests cover common axes, nonuniform scatter X, point colors, overlap, constant/extreme values,
limits/first excess, narrow reflow, theme overrides and original export. `/preview chart-series`
compares three named trends. No provider-specific data contract is introduced.

MCP `resource_link` blocks present identity, URI, optional title/MIME type/byte size,
literal description and remaining metadata. Required name/URI and typed optional fields
are validated; malformed fields or JSON above 256 KiB keep the whole literal block.
The renderer performs no resource fetch and infers no local link permission. Tests cover
80/24/80 wrapping, literal descriptions, unknown metadata, zero/u64 size boundaries,
invalid sizes, the exact/first-excess card byte limit, source and custom callback retention.
`/preview resource-link` exercises the common renderer without fetching a resource.

The exact `find` tool separates a literal glob pattern and matching-path output.
Its positive integer `details.resultLimitReached` becomes a partial-search notice;
optional `truncation` uses the same validated line/byte metadata as shell output.
Unknown keys, invalid values, or inconsistent truncation preserve the whole details
JSON. No path count, filesystem access, or clickable destination is inferred.
Frame tests cover 80/24/80 reflow, literal names, integer boundaries, invalid metadata,
source preservation and a custom renderer receiving the original typed output.
`/preview files-find` is an offline example using the common tool renderer.

The exact `grep` tool shares the search renderer and adds a content-pattern heading,
original search results, positive integer `matchLimitReached`, and boolean `linesTruncated`.
A reported match limit and shortened lines have separate notices; false adds no warning.
Other arguments (glob, case options, context) remain visible JSON. Error results stay literal
and retain their metadata. Unknown keys or invalid metadata preserve the whole details JSON.
No filename/line-number splitting, regex execution, or match-count inference occurs in TUI.
Tests cover narrow reflow, ambiguous colon paths, literal Markdown, limit boundaries, invalid
and failed results, original export and custom renderer input. `/preview content-search` is
an offline fixture. Native preview checks must restore the original tmux size policy after
fixed-width checks so the user's terminal continues to control wrapping.

The boolean truncation metadata is presented as a readable warning when true and
omitted when false; a rendered partial directory already supplies that warning.
Malformed directory output still shows the truncation warning alongside raw text.

Codex `commandExecution` uses the configurable command panel with literal aggregate
output and reported status, exit code, and duration. Streaming updates remain complete
ToolOutput snapshots; final output replaces accumulated text, while omitted/null
command, cwd, and output retain observed values. `/preview codex-shell` is offline.
Backend tests cover accumulation, sparse completion and final replacement; TUI frames
cover 80/24/80 columns, custom syntax colors, literal output and renderer callbacks.
The shared command renderer also formats reported `status`, `exitCode`, and
`durationMs` for `bash`, `powershell`, and `run_command`; this metadata is not tied
to a provider identity. Aggregate text remains literal and is not reinterpreted as
native stdout/stderr. Unsupported syntax grammars retain plain command text.
The cross-tool frame matrix checks metadata, original export, and custom callbacks.

Default shell renderers keep command/result sections and the latest five visual rows
per text output block (per stdout/stderr stream for unambiguous native results).
`tui.shell_tail_rows` configures that count; zero selects ordinary whole-tool folding.
Ctrl+O restores full output. Custom renderer output retains ordinary folding. Width,
stream updates and preference changes invalidate the normal appearance/layout cache.
Tests cover 80/24/80 columns, tabs/CJK/control/Markdown literals, count boundaries,
stream updates, full expansion, zero/max settings, custom callbacks and original export.
`/preview shell-tail` is an offline long-output fixture. Full-output-file navigation
remains separate work; native process streaming is covered below.

Shell result `details` supports the pinned pi BashToolDetails/TruncationResult shape:
reported fullOutputPath, line/byte totals and limits, and partial-line flags. Default
rendering summarizes only recognized, consistent metadata whose content is already
present in a text block. Unknown/malformed fields, paths over 4096 bytes or containing
controls, and details over 256 KiB retain literal JSON. Raw profiles and renderer
callbacks remain complete; shell tail folding leaves these notices visible. Paths
are displayed literally without file/network access. `/preview shell-truncated` is
an offline fixture. Tests cover 80/24/80 columns, line/byte/complete cases, literal
paths, raw callbacks, and exact/first-excess path and encoded metadata limits.

Native command progress now follows ToolExecution::take_progress -> the managed
backend progress admission -> a nonterminal ToolOutput snapshot. Existing hosts
and semantic policies default to no progress. Local command pipes coalesce bounded
stdout/stderr snapshots at most every 100 ms; incomplete UTF-8 tails wait for more
bytes, and finish/cancel closes pending snapshots. LocalSemanticAdmission checks
complete credentials and withholds each stream suffix matching an incomplete known
credential. Cropped or oversized raw snapshots are withheld; rejected or oversized
admitted progress follows normal failure cleanup. Only authoritative final results
enter replay and the next model request. TUI progress carries explicit progress=true,
shows separate streams without exit status, and is replaced by final output.
Tests cover a real shell blocked on a release file (output observed before exit),
UTF-8, policy opt-in/default/rejection/size/truncation, credential suffixes, cleanup,
replay and 80/24/80 TUI frames. `/preview shell-progress` is an offline timed fixture.

Native incomplete/failed model terminals retain partial answer text and reported
usage, close open activities, and do not execute unfinished calls or publish replay.
ResponseLimit emits an ActivityNotice warning through the existing customizable
notice renderer. Tests cover length/max_output_tokens during an unfinished tool call,
provider failure classification, and strict rejection of a completed terminal with
an unfinished call.

Folded shell output uses the shared grapheme placement engine with bounded trailing
row retention. Logical display-row counting uses usize while retained cell coordinates
remain u16. Full/cursor flow keeps its existing height limit and error behavior.
Tail mode preserves tabs, controls, CR/LF, wide/combined graphemes and original byte
offsets; evicted row buffers are reused. Tests cover the exact full-layout height
limit and first excess, 100,000 lines, blank trailing rows, long wrapped lines, and
actual 80/24/80 TUI frames with 70,000 lines and a 1.5-million-character line.
Original ToolOutput and callback source remain complete. Full expansion still uses the existing bounded layout. Plain export prepares items
individually and joins their glyph rows with usize positions, so the conversation
total can exceed the surface height limit. Markers, empty rows, separators and
suffix context remain unchanged. Ordinary messages and tool output use bounded
128-row pages during export, including individual items beyond the surface height
limit. Typed tools export admitted plain_text with owned headings and footers,
without visual callbacks or raw-schema fallback. Control notation and configured
body width remain effective. ModelWork plans, documents, summaries and notices also use source pages.
Document/summary interpretation and plan headings/status markers are shared with
visual rendering; plan continuation indentation and empty-line behavior are retained.
Unknown profiles remain literal source. Export no longer uses surface coordinates
to limit an individual item; retained profile size limits remain in force. Tests export over 98,000 rows across items, a 70,000-line typed
tool with a failure footer, an oversized user message through session exit, and
70,000-line plan/document/summary/notice profiles.


`/output` independently pages retained ToolCall/ToolResult text. It selects the latest
item on opening, switches tools with Left/Right, and navigates with Up/Down,
PageUp/PageDown, Home/End and the mouse wheel; F1 restores Chat. End follows appended
output until the reader moves upward. Typed profiles expose their retained plain_text,
without renderer callbacks, attachment reads, remote path resolution or execution.
Reported boolean truncation metadata keeps a `Partial output` header visible while
paging, including a compact `Partial` label or `!` at very narrow widths. A new
snapshot replaces that observation. Missing metadata, strings and file paths do not
establish truncation or prove that the retained output is complete.
The existing maximum body width and activity-body theme style apply. Unrenderable
cells use an explicitly labeled ASCII-escaped fallback; original records stay intact.
A shared grapheme scanner builds display text with sparse 128-row checkpoints and
usize row counts. Only the selected item/revision and width invalidate its cache;
scrolling lays out only the requested page. Detached navigation retains the original byte offset across width changes and
resolves it to the containing display row, while End follows the newly wrapped tail.
Per-row source offsets preserve empty lines and expanded tabs; explicit navigation
sets a new anchor. The shared `TextPages::with_escaped_fallback` constructor preserves
hard breaks and blank lines and maps escaped rows back to original source offsets.
Switching between literal and escaped pages retains the reading anchor, including
navigation inside an escape sequence. The editor and strict flow path are unchanged. Tests cover 70,000-row navigation, checkpoint and u16 boundaries, a long
single line, controls/tabs/CJK, last empty rows, source/width cache invalidation,
stream updates, failed-frame retries, local input and outstanding request correlation.
Native commands now keep a separate bounded capture for this viewer. The Rust
`NativeModelBackendConfig.maximum_retained_tool_output_bytes` defaults to 8 MiB;
`None` disables it. The host reserves framing space and splits the remaining budget
between stdout/stderr, independently of the model's 4 MiB default result budget.
Each stream keeps its head and tail if its retention budget is exceeded. Retention
therefore does not imply unlimited output or an increase in model context.

Both representations pass the existing semantic-admission gate. The larger admitted
text becomes `ToolOutput.plain_text`; the result content and model replay keep the
shorter admitted output. `retainedOutput.truncated` describes the viewer's capture
independently of `result.truncated`, which describes the model result. The default
renderer supplies an `/output` hint; custom renderers receive the same metadata.
Unknown or malformed metadata stays literal. No raw output is written to a separate
file: the admitted profile uses existing journal segments, repository capacity and
storage-pressure handling, so durable capture is subject to successful persistence.
An oversized/rejected retained result or a profile that cannot encode within 16 MiB
fails explicitly before model submission; the command is not repeated and the larger
representation is never silently replaced by its small replay result. Tests cover
exact/first-excess stream and admission bounds, credentials present only in the
middle, all terminal outcomes, model-request separation, profile capacity, and durable
segment recovery. `/preview shell-retained` provides a 120-row offline fixture.
External full-output file persistence/access remains separate work.


### Terminal hyperlinks

Markdown links retain a validated HTTP(S) destination separately from visible cells,
following the separation used in the pinned Codex terminal_hyperlinks.rs. Existing
url 2.5.8 parses absolute URLs with hosts; controls and destinations over 8 KiB are
rejected rather than emitted as terminal controls. Relative/file/executable schemes
remain visible text; this path does not resolve files or open browsers. Each destination
is shared through Arc storage, and the complete wide-grapheme footprint owns it.
Markdown span IDs preserve links through emphasis, inline code, tables and reflow;
fenced code remains literal. Destination-only changes participate in FrameDiff.

TerminalOps selects/closes OSC 8 links separately from styles and cursor geometry.
Each diff span and inline publication row closes its link. On partial-write failure,
inline/fullscreen renderers terminate an interrupted control string, close the link,
then restore cursor visibility/end synchronized update; the next attempt resets link
state before repainting. Frame-validation failures still precede new output. Tests
cover source/control/size boundaries, atomic wide-cell overwrite/clear, exact OSC bytes,
changed destinations, scrollback publication, partial failure/recovery and 80/24/80
Markdown/table frames. `tui.hyperlinks` defaults true; false removes metadata without
changing visible text/styles/source, via OutputPreferences::with_hyperlinks. CLI tests
cover false and malformed configuration. `/preview links` exercises the Rust path.
Physical click behavior depends on the outer terminal and is not proven by ANSI captures.
Bare HTTP(S) URLs in Markdown prose and tables are annotated after parsing complete
blocks/cells, before wrapping. This preserves URLs split across parser events by HTML
entities or inline emphasis. Sentence punctuation and unmatched trailing delimiters
are excluded; balanced URL parentheses and IPv6 brackets remain. Explicit link spans
(including rejected destinations) and inline/fenced code are protected from automatic
retargeting. Existing hyperlink preferences, palette and URL validation apply. Tests
cover punctuation, decoded queries, protected links/code, table reflow and updated
stream text. Explicit host-confirmed file destinations are supported below; automatic workspace-file resolution and structured citations remain gaps;
plain non-Markdown logs and input drafts do not use this annotation path.


### Host-confirmed file links

`TuiSession::with_link_resolver(Some(LinkResolver::new(...)))` lets a host resolve
explicit Markdown destinations to a validated `surface::Hyperlink`. Returning None
keeps default HTTP(S) handling. The callback is not invoked for fenced/standalone
code, images (including table images), literal tool logs, source exports or disabled
hyperlinks. Bare URL auto-detection retains its web-only path. Destinations with
controls or over 8 KiB never reach the callback. Replace the resolver handle to
invalidate cached layouts when its mapping changes; callbacks must be deterministic,
bounded and free of I/O.

`Hyperlink::from_file_path` is the explicit host-only constructor for absolute UTF-8
paths. The host must first verify file ownership, existence, workspace scope and
symlink resolution outside rendering. The constructor performs no filesystem access;
it rejects parent components, control characters, remote authorities and file URLs
larger than 8 KiB after encoding. URL metacharacters in filenames stay encoded path
data. `Hyperlink::new` continues to reject raw file URLs and executable schemes.
No file-opening command runs during rendering.

```rust,ignore
// The host has already verified known_file outside layout/rendering.
let target = Hyperlink::from_file_path(&known_file).expect("bounded authorized path");
session = session.with_link_resolver(Some(LinkResolver::new(move |destination| {
    (destination == "README.md").then(|| target.clone())
})));
```

Resolved destinations use the same cell annotations, wrapping, palette, OSC 8 encoder
and cleanup as web links. The source label and original Markdown remain unchanged.
Tests cover filename escaping, exact encoded limits and first excess, rejected paths,
callback bounds, paragraph/table reflow, resolver replacement/removal, disabled callbacks,
web fallback, source preservation and exact file-link OSC bytes.

Run the native example with `--custom-links`, then `/preview file-links`. Only the
registered README destination maps to the repository file canonicalized at startup;
unmapped files and raw file URLs remain plain. The example supplies a mapping, not a
default production workspace resolver. The outer terminal decides whether and how to
open file links; line-number jumps and physical clicks are not guaranteed by OSC output.
This follows the pinned Codex terminal_hyperlinks.rs split between general web links
and explicitly trusted file destinations.

### Session origin notice

The terminal startup path carries its confirmed new/resumed state into
TuiSessionInfo::with_startup_notice. Startup labels pair the managed provider with its
configured model, or `host:<id>` with the matching active host's reported model.
Missing or mismatched host observations show `model unreported`; account identifiers
are excluded. Successful managed and host model switches retain this execution-owner
label alongside the new model. The CLI owns this composition; the common TUI consumes
only the sanitized display value. The session displays one ephemeral info notice
with host-known backend and workspace labels, without starting a turn or adding a
journal event. It uses existing notice colors and body-width settings. Hosts can
omit the option to retain status-line labels only. Labels escape control characters
and the notice bounds each label to 4096 characters, with an ellipsis on truncation;
original status labels are unchanged. Tests cover both origins, empty/default hosts,
exact label bounds and 80/24/80 frame redraw without duplicate notices or active work.
These labels identify the configured or host-reported selection; they do not prove
per-turn model routing or remote session identity.


### Provider model rerouting

Codex `model/rerouted` notifications now display the reported fromModel, toModel and
reason as an ActivityNotice warning. The local app-server schema defines these
fields with threadId and turnId. The runtime validates the thread and active turn,
rejects missing/non-string fields and oversized notices before publication, and
ignores notifications for finished turns. The notification does not change model
picker selection or the eventual turn outcome. Existing warning colors, body width
and source export apply. Tests cover completion/failure ordering, late events,
invalid targets/fields/size, and custom warning presentation. The reason is preserved
as reported, including unknown string values; yo does not infer why routing occurred.
`/preview reroute` displays an offline example without changing models.


### Turn aggregate changes

Codex `turn/diff/updated` carries the latest unified diff across the turn, according
to the local app-server schema. The runtime retains one FileChange activity per
turn, replaces its snapshot on each update, and labels it `Turn aggregate diff` to
distinguish it from individual tool changes. Empty updates remove the previous diff
and explicitly report no remaining changes. The activity closes with the turn's
completed/failed/interrupted outcome; finished-turn updates are ignored. Thread,
turn, string type and the presentation byte limit are validated before publication.
Existing diff colors, folding, width and `/changes` file navigation apply. A leading
explanation stays with the first file rather than creating a separate file section.
Tests cover replacement/empty updates/all terminal outcomes, invalid targets/types,
the exact size limit and first excess, and file navigation across 80/24/80 columns.
`/preview turn-diff` is an offline two-file example. No file reads or inferred diffs
are performed, and individual tool changes remain separate observations.


### Compact usage observations

Completed, validated usage receipts use a compact input/output line when activity folding
is enabled. Ctrl+O restores cache, reasoning and reported context-window details. The
compact line wraps at narrow widths and uses the existing muted theme role. Only the
presentation of a successfully completed receipt is tagged; arbitrary lookalike text,
invalid receipts, interrupted and failed observations retain their existing display.
Every receipt remains a separate activity and full plain export retains all details.
No backend events, receipt schemas or session usage accounting rules change. Tests cover
Codex and Grok observations together, both individual counts, 80/24 resizing, folding
round trips and unchanged source. This reduces each panel rather than merging receipts.

### Assistant answer rendering

Assistant-answer presentation uses `AssistantRenderer` / `TuiSession::with_assistant_renderer`.
The callback receives original body, available columns and whether the item is finalized
(not a success claim). User/tool/approval/document content and plain exports are excluded.
TUI-owned failure/interruption text is retained as literal text outside replacement Markdown.
`None`, more than 256 KiB, or layout/combined-footer height failure restores the original body.
Handle identity invalidates layout caches; callbacks must be deterministic, bounded and free
of I/O. Theme/output preferences still apply. `--custom-answers` customizes the `markdown`
Rust preview and displays its current width and streaming/final state.

Non-text assistant output uses the backend-neutral `MessageContent` profile. Grok ACP
image/resource/unknown answer blocks retain original fields and split the surrounding
text stream in source order. The common TUI shares tool media rendering, image settings,
and the assistant renderer callback (whose source contains the original profile).
Plain export retains the original block JSON, including metadata, and failure footers.
No resource URI is fetched. `/preview message-image` exercises this path offline.

Provider-delivered reasoning uses `ActivityReasoning` (`yo.activity-reasoning/v1`),
separate from public summaries. Grok ACP thought text accumulates within its message;
non-text blocks retain the original JSON and split the text stream in order.
The common view labels this “Agent reasoning”, applies `show_reasoning` and reasoning
colors, and preserves source and failure footers. DocumentRenderer receives the
original text or a literal JSON fence; hidden bodies remain hidden even with a
custom renderer. Non-text content grants no image or attachment authority.
`/preview agent-reasoning` exercises this profile without a model call.
Profiles are limited to16MiB; oversized Grok reasoning fails explicitly rather than
falling back to untyped text that bypasses the visibility setting.

### Custom document rendering

`DocumentRenderer` and `TuiSession::with_document_renderer` customize only explicit
ActivityDocument bodies and document projections of ActivityReasoning carried by ModelWork. The callback receives the original typed
document, observed terminal outcome, expansion preference and effective body width.
Returning Markdown uses the existing code/table/chart renderers; None keeps the original.
Titles, status/failure footers and plain export remain host-owned. Summaries, tools and
approval requests never enter this callback. Document images remain placeholders and
never grant attachment authority. Results above 16 MiB or unrenderable layouts fall back
to the original body. Empty custom bodies remain empty; source is retained independently.

As with ToolRenderer, callbacks must be deterministic, bounded and free of I/O. Replace
the handle to invalidate caches; mutable captured state alone does not trigger layout.
Tests cover typed input/outcome, repeated frames, Ctrl+O, narrow reflow, preserved failure
and source, callback replacement/removal and layout failure fallback. Run the standalone
Rust example with `--custom-documents`, then `/preview terminal-wait`, to inspect compact
and expanded host-provided presentation. This does not yet provide arbitrary interactive
widgets or a session-level status/document channel.

### Grok structured tool results

Grok ACP ToolResult activities now use the common ToolOutput profile. The adapter
retains rawInput/rawOutput/content/locations/_meta field replacements across partial
updates; omitted fields remain, explicit empty content clears prior content. Tool
names use the reported name or call ID, without guessing from a title. Arguments are
available to custom tool renderers and readable source output. Plain ACP content
wrappers map to their original inner text/image/resource blocks; wrappers with extra
fields and terminal entries remain literal JSON. No terminal/file I/O is added.
Raw output and metadata remain under their original keys. A failed status keeps the
existing failed activity outcome and footer, without treating arbitrary result fields
as errors. Oversized profiles retain literal fields rather than losing their source.
Terminal calls release the extra accumulated output; original retained result events
remain available. Tests cover approval identity, partial fields, image/diff preservation,
explicit clearing, changed arguments, failure and cleanup. Physical image support and
real Grok service behavior still need separate terminal/authenticated verification.

ACP diff entries become generic `type: "diff"` content with a literal `title`, unified
`text`, and the complete original entry in `source`. The adapter compares oldText and
newText with three context lines; null oldText denotes a new file. Combined input
above 256 KiB or invalid fields remains literal JSON. Diff calculation has a 50 ms
search timeout; approximate hunks may result. Paths are quoted in patch headers.
The common TUI applies diff colors and wrapping, with no provider-specific branch;
empty diffs say `No textual changes`. Custom tool renderers retain source access.
`/preview tool-diff` exercises this common content in the Rust preview without a model
call or file write. Tests cover multiple hunks, new/unchanged files, path escaping,
size boundaries, narrow resizing, theme roles and source export. For calls initially
classified as edit/delete/move, the same FileChange activity now also receives the
reported diffs, with explicit quoted file headers for `/changes` navigation. Partial
updates retain those files; explicit empty content clears them. Original call failure
and interruption remain authoritative; a displayed patch does not establish that a
write succeeded. Other tool kinds keep their existing classification. No additional
file activity or local file read is created. Tests cover multiple files, escaped paths,
partial updates, clearing and failure on the same activity. Authenticated service
behavior still requires separate verification.

### Grok approval choices

ACP permission options now use the common ActivityApproval layer in their original
order. Known allow/reject once and allow/reject always kinds map to their exact option
IDs; unknown kinds remain visible but disabled. Remembered-choice scope belongs to the
agent, and a successful response does not prove its policy was persisted. Legacy binary
approval still selects only allow_once/reject_once. The safe default is reject_once.
Duplicate IDs, more than 64 choices and invalid identifiers fail before publication;
profiles above 16 MiB are rejected using the reported reject_once option. Invalid,
disabled or out-of-range ordinals do not consume the request or send a response.

The readable request includes the reported tool call ID and rawInput arguments, without
copying arbitrary tool metadata into the semantic approval. A known unfinished file
change in the same Turn links through the exact toolCallId to the existing `/changes`
review gate. Other tool kinds and finished calls remain unlinked. Requests arriving
before the matching call retain its bounded ID; a later unfinished FileChange updates
the same approval profile, in request order, without changing choices or response authority.
Admission reserves the maximum related ID encoding, including the exact byte boundary.
Unknown IDs alone are insufficient for approval, but an ID-only request can use the
same Turn's unfinished call title or name/input summary and arguments. Explicit new
arguments override retained arguments, including in a reconstructed summary. Unrelated
calls never supply fallback details or links. Existing TUI profile-update and fresh-frame
checks invalidate old review receipts when a late link or related diff appears.
For an exact known unfinished call, permission-request rawInput/content/locations
refresh the retained tool snapshot before the approval is published. New reported diff
content therefore replaces the older `/changes` body, and explicit empty content clears
it. Permission status/kind does not finish or reclassify the tool; rawOutput and arbitrary
metadata are not imported from the permission request. Validation and duplicate-request
checks precede this refresh. Request content and locations also remain literal in the approval history. For an
unknown bounded call ID, the pending approval retains those display fields within the
same profile byte bound. When the actual call arrives, its explicit fields (including
empty or null content) win; only omitted fields use the newest pending request that
provided them. Matching uses exact ID and Turn, without synthesizing a tool activity.
The pending copy is released after projection or request cleanup; original request
history remains. The resulting tool snapshot uses the normal rich diff renderer. Tests observe the updated file body
and preserved ToolOutput source before ApprovalRequest, including clearing and a misleading
completed status in the request. Response receipts retain
the selected label and scope only after the wire write succeeds, without repeating
arguments. Tests exercise all four supported kinds, original ordinals, disabled and
invalid choices, duplicate IDs, 64/65 choice bounds, oversized rejection, exact file
identity and one-time response consumption. Real agent policy persistence and terminal
approval interaction need authenticated verification. See [ACP permissions](https://agentclientprotocol.com/protocol/v1/tool-calls).

### Grok ACP plans

The Grok adapter maps [ACP v1 plans](https://agentclientprotocol.com/protocol/v1/agent-plan)
to the common ActivityPlan snapshot. Every update replaces the complete ordered list;
priority is retained as a literal `[high]`, `[medium]` or `[low]` prefix. Empty lists
remain explicit. One plan activity persists across intervening tools within a Turn,
counts against the existing active-activity limit, and closes with the Turn outcome.
A later Turn gets a new activity. Completion/interruption never changes reported
step statuses. Invalid fields/statuses/priorities and oversized encoded snapshots
are rejected before publishing a partial plan. Tests exercise replacement, empty
plans, tool interleaving, interruption, next-Turn identity and exact byte boundaries.
The TUI uses its existing plan palette/layout; no Grok-specific rendering is added.

### Managed tool approval details

The managed backend emits readable approval text with tool name, per-call scope,
effect, execution host, call/tool IDs and the bound argument digest. Arguments come
only from the exact semantically admitted replay entry for that call, never from raw
execution arguments; the UI labels this the recorded view. Redacted replacements
remain redacted. The existing Approved/Declined responses and exact execution binding
are unchanged; this does not enable offered choices or persistent grants. Tests check
that details appear before execution, approval permits only one attempt, and raw
arguments do not leak through a redacting admission policy.

### Selectable interview choices

ActivityQuestion (`yo.activity-question/v1`) carries a readable prompt and ordered
QuestionChoice labels/descriptions for an active UserInputRequest. Codex sequential
questions emit this optional profile; the existing numeric-to-label response mapping
and request IDs remain authoritative. A 64-choice and ToolOutput snapshot byte bound
apply; larger presentations retain the existing plain-text question path. The TUI
keeps readable prompt text in Chat/export and shows choices in the request panel.
Chat/export also append a literal Choices list with original one-based ordinals, full
labels and multiline descriptions for both question and approval profiles. Disabled
approval choices are marked unavailable. Descriptions need not appear in plain_text;
completed requests retain them after the panel closes. Raw Transcript records remain
unchanged, and matching JSON inside ordinary tool logs is not interpreted as a request.
Interview and approval panels use spare height (up to six rows) to wrap the selected
choice's label and description beneath the list using the existing detail palette.
Up/Down refreshes this detail; limited height shows an ellipsis and PgUp full-details hint.
PageUp/PageDown scroll the readable Chat request while leaving the choice pending.
Choice rows retain priority when the terminal is short.
F2/F3 history round-trips preserve selection only for the same unchanged request.
Returning opens a fresh panel token: a previously committed frame cannot authorize
submission. A request update while away discards the saved selection. F2 currently
shows the complete structured payload as escaped JSON, not a formatted details page. The transcript retains the
complete request; this detail area does not change choice ordinals or approval scope.
Up/Down selects, Enter answers only after the panel has been presented, and typing
an answer keeps the free-text route. Changed choices replace the panel token; late or
unrelated requests cannot use its acceptance receipt. Existing panel colors, focus,
wrapping and descriptions apply. `/preview interview` uses the same profile for both
questions. Tests cover exact size/count bounds, ordinal/label handoff, presentation
before acceptance, narrow frames and readable export. Secret input remains unsupported;
Codex's isOther flag adds “None of the above” only when options are nonempty, matching
the pinned request_user_input overlay's options_len_for_question/option_label_for_index.
Selecting it returns that label through the same request ID. Absent/false flags do not
add it, malformed flags are rejected, and free-text input remains available independently.
The interview preview demonstrates it in its first question.

Hosts opt into choice-plus-notes with ActivityQuestion.allow_notes (default false for
older profiles). Codex and the offline interview opt in. Tab on a presented choice
opens a notes panel showing its label/description; Enter submits the selected ordinal
and optional notes together, and Tab returns to choices without discarding the draft.
The same panel styles and width handling apply. Slash-prefixed and multiline notes
remain literal answer text. A changed question clears the attached selection; a new
panel must be presented before accepting it. Free-text-only answers keep their existing
route. AgentIntent::RespondToQuestion and ActivityResponse::QuestionAnswer retain the
original request identity, choice and exact structured notes through admission and
journal round trips, without another SubmissionId. The Codex adapter validates the
one-based choice against its original options before sending anything, then emits the
original label and, when nonblank, a separate `user_note: <trimmed notes>` answer, matching
the pinned request_user_input renderer. Empty notes emit only the label. Tests cover
capability defaults, invalid choice zero/first excess/u32 maximum, exact wire IDs,
sequential replies, stale presentation, 24-column frames, literal notes and journal
codec round trips. Secret input and returning to previously submitted questions remain gaps.

### Host status line

The CLI installs a session-owned host presentation connection for all currently local
backends. It refreshes the observed Git branch outside the TUI, waiting five seconds between collections;
unborn branches retain their name, detached HEAD is explicit, and failed or invalid
observations display `Git status unavailable`. Git environment overrides are cleared.
Each process has a two-second deadline and1024-byte output limit; shutdown cancels
the wait and kills/reaps the owned process group. Host snapshots coalesce, alternate
with source polls, and never replace source errors or closure. The worker survives
terminal reentry and stops with the session. The same connection delivers fresh `AgentPoll::Links` handles. Core local-workspace
code owns discovery and resolved containment: Git uses cached/untracked files with
effective standard excludes and fsmonitor disabled; plain directories use bounded
non-symlink traversal. Directory descriptors pin the root and resolve every queued
component with `openat(O_DIRECTORY | O_NOFOLLOW)` before enumeration. The regression
replaces both a queued directory and its ancestor with outside symlinks and requires
failure before reading outside entries. Discovery is capped at8192entries/4MiB; filesystem validation
checks cancellation and a two-second deadline between operations. Individual filesystem
syscalls still follow OS latency. Unknown, changed, deleted, symlinked and outside-root
paths are omitted. Failed/oversized scans clear the mapping. Relative, `./` and exact
canonical absolute destinations map to host-authorized file URLs. Percent escapes are
decoded exactly once against that inventory; plus stays literal and malformed escapes
remain unresolved. Raw file URLs and
unmapped destinations retain web-only fallback. Every update replaces the immutable
resolver and invalidates layout; source export and code remain literal. Links are
navigation, not attachment or submission authority, and can change between refreshes.
Only currently supported local execution workspaces use this producer; this does not
map remote workspace paths into the frontend filesystem.

`TuiStatusLine::new` accepts a complete snapshot of up to 16 unique keys and display
values. Keys are nonempty, control-free and at most 64 bytes; each value is at most
1024 bytes both before and after visible control escaping. Values are sorted by key,
joined with separators, and empty values omitted. Duplicate keys, the first excess
entry/byte, and text that cannot form one display line return `TuiStatusError` before
an update is constructed. `TuiStatusLine::default()` clears the status.

Hosts can call `TuiSession::set_status_line` before terminal ownership/reentry, or
emit `AgentPoll::StatusLine(status)` during a live run using their existing readiness
notification. This is a host presentation observation, separate from provider wire
protocols, Turns, message output, usage accounting and the session journal. Hosts own
aggregation and replacement when the underlying context changes. Identical live
snapshots do not request a redraw. Terminal reentry retains the session-local value;
entering the offline preview uses its separate state.

The line sits between metrics and keyboard help, uses the existing `muted` semantic
color and reserves one row only after prompt/help and the transcript floor. At narrow
widths it preserves complete graphemes and uses ASCII dots to mark omission; insufficient
height hides the optional row. Source export is unchanged. This provides an opt-in plain
status line, not arbitrary interactive widgets or a custom replacement of permission UI.

```rust,ignore
let status = TuiStatusLine::new([
    ("checks", "Tests: 18 passed"),
    ("worker", "Worker: ready"),
])?;
// Return from a host AgentConnection::poll implementation:
Ok(AgentPoll::StatusLine(status))
```

The pinned pi footer extension-status loop sorts keyed statuses, sanitizes them and
adds a width-limited row. `/preview status`, `status-update` and `status-clear` exercise
the common yo host-poll path without creating a model Turn or transcript record.
Tests cover limits, control escaping, canonical ordering, 80/24/3/80 frames, live
replacement/removal, semantic color changes, source preservation and heights 0–30.

### Prompt line editing

The shared prompt editor supports Ctrl+A/E for logical line start/end, Ctrl+U/K
for deleting toward the start/end, and Ctrl+Y for reinserting the latest deleted
text at the cursor. Repeated deletion at a line boundary removes the newline and
joins adjacent lines. Consecutive backward deletions prepend and forward deletions
append to the retained text; typing, cursor movement, pasting or replacing a draft
starts a new deletion sequence. This is an editor-local last-deletion buffer, not
a system clipboard, an undo history or a rotating kill ring.

Logical LF/CRLF boundaries determine the range; visual wrapping and terminal width
do not. Grapheme-aware boundaries preserve CJK, combining marks and emoji sequences.
Release events and extra modifiers never execute these bindings. The same editor
handles normal messages and interview notes without submitting or interrupting a Turn.
Home/End retain their conversation navigation roles.

Compare the pinned pi `packages/tui/src/components/editor.ts` deleteToStartOfLine /
deleteToEndOfLine bodies and Codex `bottom_pane/textarea.rs` kill action dispatch.
Tests cover Unicode text, consecutive deletion order, newline joins, CRLF atomicity,
24/80-column layout and restored interview answers. In `/preview interview`, use Tab
to add a note, answer, Shift+Tab back, Ctrl+U to replace that note, and verify the
edited value after final submission.

### Revisiting interview questions

`ActivityQuestion` optionally carries `previous_question`, `draft`, and `draft_choice`.
Missing fields disable backwards navigation and leave the editor untouched. A restored
choice must be a valid one-based ordinal with notes support and a present draft; invalid
profiles take the existing literal fallback. The serialized profile retains its existing
16 MiB limit.

On a supporting question, **Shift+Tab** retains the current text or choice-plus-notes
and returns to the previous question. The common `PreviousQuestion` intent/response
carries the exact outstanding request ID and the unsubmitted draft through admission,
backpressure and the journal codec. The TUI requires a freshly presented request panel,
restores drafts once into an untouched editor, and never overwrites later typing on
snapshot refresh. Restored answers require a fresh frame before Enter; slash-prefixed
restored text stays an answer rather than invoking a local command. Ordinary questions
without this capability keep their existing input behavior.

Codex keeps the batch and drafts in its adapter, issues a fresh correlated request on
each move, and sends no JSON-RPC answer for navigation. Re-answering replaces that
question's recorded value; the final question sends the complete updated answer map.
The first question and stale or out-of-range navigation are rejected. A previous draft
that cannot fit the bounded question profile disables navigation to it. Grok does not
offer this feature and rejects this response through its existing unsupported-input path.
The common TUI never reads Codex wire fields or batch state.

`/preview interview` supports the same gesture with isolated drafts and fresh request
IDs. Exercise first answer → second draft → Shift+Tab → edit first answer → restored
second draft → final submission, including 80/24/80 resize. Regression tests cover
provider payloads, stale requests, optional-profile validation, draft restoration,
source command round-trips and fresh-frame gating. Restarting a live provider interview
and encrypted/secret drafts are not established by these tests.

### Recorded interview answers

Codex question prompts state the submission boundary before input: intermediate
answers are recorded locally, and submitting the final question sends every answer.
A single-question prompt says that submitting sends its response. This adapter-owned
text uses the existing common question presentation; other providers retain their own
submission semantics. Supported questions can now be revisited as described below.

Codex user-input response activities now retain the question, resolved option or
literal answer, and separately labelled notes. Intermediate responses say that they
are recorded while remaining questions are pending; the final receipt is queued only
after the JSON-RPC response write succeeds. A failed write creates no sent receipt
and leaves the request unanswered. The wire answer array remains unchanged. The
receipt is bounded by ToolOutput::MAX_SNAPSHOT_BYTES before allocation; an oversized
receipt explicitly omits display content while the original response remains in the
journal. Exact-limit and first-excess answer/note cases are tested.

The TUI retains these typed UserInputResponse activities with an “Answer recorded”
heading using existing activity success/failure colors, body style and body-width
preferences. Responses bypass generic tool folding so a narrow frame does not hide
the submitted answer. Raw Markdown, image and link strings remain literal; tool-body
callbacks never handle them. Failed/interrupted footers and complete plain export are
preserved. Tests cover 80/24/80 frames, custom colors, literal text, export, sequential
question IDs, partial interview interruption and response-write failure. The offline
interview preview emits the same activity kind and labels its local receipt as offline.
This follows the pinned Codex history_cell/request_user_input.rs field separation;
secret input remains separate work.

### Incomplete interview summaries

When a pending interview is interrupted, its Turn fails/ends, or the agent closes
the request, the adapter emits one warning notice with recorded/total and unanswered
counts plus every question marked Recorded or Unanswered. Counts come from retained
answers rather than menu selection or an editor draft. Submission remains explicitly
incomplete. Already sent bundles produce no incomplete warning. Request bindings are
removed on closure; late resolved notifications and responses cannot duplicate the
summary or revive the question. A normal Turn completion with unanswered requests
still fails the core RequestStillUnanswered check; presentation does not fabricate an
answer to make that invalid completion succeed.

Connection EOF and receive failures also close request bindings and deliver these
summaries before the runtime handles the terminal failure. The original receive
failure is preserved; repeated polling does not read the closed connection again or
duplicate notices. A failed response write still produces no sent receipt. Its error
retains the original failure kind/message and adds the number of earlier recorded
answers and total questions, stating that final submission was not confirmed. It does
not echo answers or notes or assume a failed write proves the peer received no bytes.
The final attempted answer is not counted as an earlier recorded answer.

The existing ActivityNotice warning/body palette and width settings apply, with literal
question text. The complete serialized notice fits the existing output snapshot limit;
if the question list exceeds it, the summary explicitly omits that list while retaining
counts and closure context. Tests cover exact maximum/first excess, zero/partial answers,
all remote closure paths, successful send without a warning, duplicates and stale replies.
The native offline interview also emits a warning on cancellation, preserving prior
answer receipts; cancelling before the first answer and after one answer are tested.

Chat supports `Alt+Up/Down` to move between rendered item starts. From the middle of an item, Up returns to its start; Down beyond the final start resumes tail following. This uses current-width layout boundaries and preserves prompt drafts. Test both 80 and 24 columns and multiple keys before a committed frame. `Alt+O` toggles the committed frame’s context activity: first visible while detached, latest while following the tail. It ignores pending navigation until a frame commits. Per-item overrides survive resize and feed ToolRenderer/DocumentRenderer expanded inputs. Ctrl+O clears overrides and toggles the global state. Inline publication pauses while an override exists so persistent output cannot discard that choice. Source export is unchanged.

The `histogram` fence accepts 1–64 finite whitespace-separated numbers. Optional first line `bins: N` selects 1–32 equal-width intervals (default 8). Counts use left-inclusive/right-exclusive ranges, with the maximum included in the final interval. Constant input uses one bin; duplicate floating-point edges reduce the effective bin count. Labels use round-trippable numeric representations, and original numeric tokens remain visible. Invalid options, nonfinite values and the 65th sample retain source. Rendering reuses chart bar styles, monochrome fallback and responsive bar layout. `/preview` → `histogram` provides an offline native fixture. Check boundary counts, bins32/33, samples64/65, constant/subnormal/overflowing ranges and80/24/12 reflow. This follows the interval-count presentation described in [plotext’s histogram guide](https://plotext.readthedocs.io/en/latest/bar.html); no Python dependency is introduced.

`scatterchart` accepts 1–64 whitespace-separated `x,y` pairs, with both coordinates finite. Numeric X spacing is independent of source order; points are not connected. The existing Braille chart surface, palette and ASCII fallback apply. X/Y ranges and original coordinate tokens stay visible. Constant axes place points at the center; extreme finite ranges use bounded normalization. When axes cannot fit, the display retains ranges and a coordinate list. Invalid pairs or the 65th point preserve source. `/preview` → `scatter` exercises the native Rust path. Tests inspect unconnected dots, nonuniform X spacing,80/24/12 reflow, constants/subnormal/extreme axes and64/65 limits. [plotext basic plots](https://plotext.readthedocs.io/en/latest/basic.html) informed the scatter/connected-line distinction.

The first line of `linechart`, `stepchart` or `scatterchart` may specify `height: N` (2–16 plot rows; default 6). Axes, ranges and original numeric tokens remain visible; annotation rows are separate from plot height. Invalid or out-of-range height retains source. Narrow-width fallback remains unchanged and does not allocate the requested plot. This is per-fence customization, not a provider setting. `/preview` → `chart-heights` compares 3-row line and10-row scatter plots. Tests count actual axis rows at80/24 columns, check12-column fallback, and verify the16/17 boundary and malformed options.

Hosts can emit `AgentPoll::Document(TuiDocument::new(ActivityDocument { title, markdown }).expect("validated document"))` without a model Turn. TuiDocument is immutable/shared and validates the existing nonempty-title and16MiB encoded profile limit before admission. The live poll and offline preview route append an ephemeral typed document to Chat; no core event, backend input or Journal record is fabricated. Existing DocumentRenderer, theme, per-item expansion and source export apply. Repeated polls append distinct entries; hosts own deduplication, and process restart does not automatically recover these documents. `/preview` → `session-document` shows a table/code guide. Tests verify the exact live apply_agent_poll handoff, no active Turn, custom expanded rendering at80/24/80, source retention, exact size/first excess and preview output containing only submission acknowledgement/document.

`TuiDocument::with_expanded(bool)` chooses an initial state for that document. Omission inherits the global activity expansion state. The option uses the existing item-identity override and inline-publication guard; Alt+O and Ctrl+O remain user controls. It never changes Markdown or source export. The session-document preview starts expanded so its table and code are immediately visible. Tests cover both global states, omitted/false/true initial settings and subsequent user override at80/24/80.

The real `/help` command now emits an initially expanded TuiDocument. Command entries come from the existing registry; command/help.rs owns the reading/editing/approval/interview guidance. Existing DocumentRenderer and theme apply. It remains local, clears the command draft, starts no Turn and does not answer pending requests. Tests verify every registered command, key guidance,80/24/80 start/end navigation, no folded rows and unchanged source. Validate with `/help` outside offline preview as well as during pending request tests.
