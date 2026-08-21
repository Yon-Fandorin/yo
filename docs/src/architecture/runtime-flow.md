# Runtime flow

Use these traces when a change crosses crate boundaries or when an error
message does not make its owner obvious. They describe the current
implementation path. Methexis remains the authority for what each boundary
must mean.

## Prospective activation review

One later independent activation can be reviewed before it becomes trusted:

```text
exact activation request in a clean candidate worktree
  ↓ require trusted v1alpha3 capability + exact four-path activation-only diff
  ↓ validate trusted develop basis + predecessor CAS + approved Checkpoint
review-only prospective ContextBuild
  ↓ same deterministic BuildId as that exact Checkpoint after activation
v1alpha3 review packet
  ↓ bind request + proposed Checkpoint + proposed active record + complete diff
prospective review evidence only; no activation or general eligibility
```

Ordinary candidates continue to use the active-authority packet route. The
prospective route never infers a proposal, falls back to active authority, or
reviews a candidate that changes its own implementation or workflow.

## Model-service and OpenAI-compatible connectors

The provider-neutral service inputs, explicit remote API dialect, and Yo-managed
loop form one typed route:

```text
configured ProviderId + AccountId + normalized endpoint
  ↓ optional base profile + whole-field model overrides
complete EffectiveModelProfile for one ModelId
  ↓ exact ModelCatalog namespace lookup
EffectiveModelBinding
  ├── explicit ApiDialect → exactly one built-in ConnectorId
  └── normalized HTTPS base endpoint
ModelContextProfile
  ↓ optional known hard output cap + injected tokenizer profile
NativeModelBackend checks retained replay + the current Turn delta
  ↓ rebuild and count the exact final connector payload for this round
positive request-local cap within a known hard max, or omitted unknown cap
  ↓ known: exact input + cap <= input limit; unknown: exact input < input limit
admitted connector dispatch

credentials.yaml beside the selected config path
  ↓ one no-follow handle; regular file, current owner, 0600-equivalent,
    bounded size, stable metadata
immutable CredentialStore
  ↓ exact ProviderId + AccountId lookup
redacted ApiCredential
  ↓ yo-cli exact identity+dialect composition
external OpenAiResponsesConnector, OpenAiChatCompletionsConnector, or KimiChatCompletionsConnector
POST <normalized base>/responses
  or <normalized base>/chat/completions
  ↓ bearer auth + same-origin bounded redirects + finite deadlines
bounded dialect-specific text/event-stream decoder
  ├── correlated text, visible refusal, and optional reasoning deltas
  ├── exact function call identity, name, and argument bytes
  └── completed, incomplete, or failed terminal + usage
  ↓ NativeModelBackend
semantic ModelWork and ToolCall Activities
  ↓ frozen ToolRegistry schema validation, host semantic-admission gate,
    and exact approval binding
injected ToolExecutionHost, one serial execution attempt
  ↓ bounded output passes the same semantic-admission boundary
durable ToolResult Activity before another remote request
next model round or resumable semantic replay delta
```

`yo-core::model_service` owns this resolution and validation. A missing
credential file produces an empty snapshot without creating anything; an
existing unsafe or malformed file fails closed. API keys have no environment
fallback and diagnostic formatting never exposes their contents. Display names
remain optional metadata and never participate in identity or routing.
`yo-core::model_connector` owns the neutral port and derives one built-in
connector identity from `api_dialect`; `yo-cli` maps that exact identity and
dialect to the independent Responses, Chat Completions, or Kimi crate without
Provider probing or fallback. Responses appends exactly one `responses`
segment; Chat Completions and Kimi append exactly `chat/completions`. No route adds
another `v1` or enables provider conversation authority or built-in tools. The Chat decoder
requires one index-zero choice, finish then final usage then `[DONE]`, preserves
content and refusal independently, and correlates indexed tool-call fragments.
A tool call's first fragment fixes one non-empty ID and function name. Later
fragments may omit them; an explicit empty repeated ID from a compatible API is
normalized as omission, while a different non-empty ID or function name still
fails the stream.
Both dialects bound SSE events and cumulative payloads while reading, and
cancellation interrupts header, stream, and queue waits. The default agent
route has no absolute model-request deadline. It still bounds connection setup
at 30 seconds, each redirect attempt's response headers at 5 minutes,
successful-stream inactivity at 5 minutes, non-success error-body inactivity at
30 seconds, and each internal event handoff at 5 minutes. Only a non-empty raw HTTP body chunk resets a body
inactivity clock, before SSE decoding or error-body retention; each observation
starts a fresh event-handoff wait. `yo-core::backend::native` owns semantic Activities and the bounded
model/tool loop. Before each dispatch it counts the exact request with the
catalog-selected tokenizer profile after checking the retained replay prefix
plus the current Turn delta. For a known hard output maximum, it makes a finite,
strictly decreasing selection of a positive request-local cap at or below that
maximum and rebuilds and recounts every candidate payload. For an unknown
maximum, the connector payload omits the cap and its exact input count must stay
strictly below the input limit. Any capacity failure before a final assistant
semantic/private delta is applied records a Failed Turn with
`code=context_exhausted`, creates no continuation anchor, makes no over-budget
remote request, and latches the binding against later Turns. Only capacity
exhaustion while applying an otherwise valid final assistant delta completes
that current Turn without resumable evidence or an anchor. Raw tool
arguments are schema-validated and tool output is bounded before the injected
host gate decides the semantic form allowed into Activities, replay, and later
requests. The backend records only that admitted call/result replay, defers an
approved effect until its approval and attempt Activities can be journaled, and
attributes each terminal response's usage to its exact Provider, Account,
Model, connector, endpoint, and complete resolved profile. The process host owns startup
selection and assembly of these inputs and concrete local tools.

For a new local-tools Session, startup freezes the five-tool basic registry in
the order `list_files`, `read_files`, `edit_file`, `write_file`, and
`run_command`. Resume compares the durable replay projection with the exact
basic, preceding three-tool legacy, and empty manifests; an unknown or mixed
projection goes to the existing read-only failure path. A later model-binding
replacement carries the already selected registry revision instead of silently
upgrading the Session's tool history.

The file host validates concrete item, numeric, path, and content bounds in the
semantic-admission path before an execution attempt and repeats defensive
parsing before opening a path. `read_files` captures each regular UTF-8 file
independently through the retained workspace directory descriptor and returns
ordered compact-JSON windows or one bounded error per item. `edit_file`
computes every unique non-overlapping match against one captured original;
`write_file` supplies one complete file image. Both mutation tools serialize
inside one host instance, write a same-parent owner-only scratch file, verify
its retained identity, and publish with one rename. Failures close and remove
only still-owned scratch state; other processes and editors remain explicit
last-publisher-wins actors rather than participants in that in-memory lock.

The local `run_command` host treats every non-empty stdout or stderr chunk as
one shared progress signal. Its default attempt has a 5-minute output-inactivity
window and no absolute execution deadline. A runtime policy may add one
absolute deadline to the execution request; that clock starts once and output
does not reset it. Inactivity, the optional absolute deadline, and cancellation
all enter the same finite process-group termination, child reap, and output
drain path, while diagnostics keep those causes and cleanup failure distinct.
A dedicated waiter owns the child from spawn through its one eventual wait, so
the bounded result path may report cleanup failure without abandoning a child
that becomes waitable later. Independent stdout and stderr readers continue
draining after their retention caps, keep only bounded head and tail views with
an omitted-byte marker, and therefore do not turn output truncation into
`EPIPE` or a changed command effect. If a writer outside the command process
group keeps a pipe open past the cleanup grace, one explicit shutdown wake
closes both local read ends and joins both reader threads; that attempt reports
cleanup failure instead of retaining thread or descriptor ownership
indefinitely. The host never retries the command automatically.

Every opened backend binding declares its continuation strategy. The current
Yo-managed route declares exact replay by the local client; Codex declares
backend-managed state. Exact replay commits a separate bounded
`ModelReplayDelta`, a payload-free resumable outcome that points to that delta,
and then the Continuation Anchor. Backend-managed state commits an outcome with
no replay-delta reference before its Anchor. Recovery validates this ordering
and strategy-dependent presence instead of inferring ownership from backend
names. A managed-server exact-replay executor remains a reserved contract value
and no current backend selects it.

### Model selection and replacement

Startup accepts one optional `--model TARGET_REFERENCE`. Exact `host:codex`
selects the Local Codex HostTarget. ModelTargets use `Model`, `Provider::Model`,
or `Provider:Account:Model`; Provider and Account encode `%` as `%25` and `:` as
`%3A`, while the vendor-owned Model suffix remains unchanged. Matching is
derived from configured complete coordinates rather than separator precedence,
so vendor ModelIds may contain `:`, `/`, or `.`. A bare model reference stays
inside the current Provider and Account, or requires one globally unique exact
ModelId when no namespace exists. The two qualified forms require respectively
one exact Provider-and-Model account match or one exact complete coordinate.
Absence and ambiguity fail with stable, sorted canonical complete coordinates.

For a new Session, an explicit invocation target overrides the stored
`connections.yaml` preference and the policy default, in that order.
`config.yaml` contributes no model target. When all selectable layers are absent,
startup fails before Session creation with exact `yo connect` and `yo --model
host:codex` guidance instead of silently choosing Codex. `yo default TARGET`
admits and stores one exact HostTarget or configured ModelTarget, while `yo
default --unset` clears only this stored layer. A resumed Yo-managed Session
does not read the stored preference and uses its newest durable binding as the bare
namespace; startup defaults never replace it. Exact `host:codex` confirms a
Codex resume, while a different cross-backend target fails explicitly because
cross-backend handoff remains deferred.

While a Yo-managed TUI is idle, `/model` opens the generic selection panel with
entries ordered as Provider, Account, then Model. Labels use the optional
display names, but each row carries the complete stable coordinate. `/model
MODEL_REFERENCE` uses the same resolver as startup, so its bare form remains in
the current namespace while a qualified form can select another configured
Provider or Account. A Codex-started live Session does not expose this picker.

The frontend-neutral `ModelSelectionController` owns those resolution rules.
After acceptance, the process host constructs and validates the candidate
backend from the startup credential snapshot, tokenizer, connector, tool
registry, and tool host while the current binding remains live. A preparation
failure is reported back into the retained TUI. The Session worker then commits
the exact-replay transition atomically: a durable failure discards the candidate
and keeps the old backend usable, while success swaps the backend in place after
closing the prior binding epoch and opening one replacement epoch. The same TUI
and Yo Session remain active, and the choice does not change configuration
defaults.

## Startup

The terminal is acquired only after process policy and the agent Session are
ready:

```text
yo-cli
  parse presentation mode, glyph profile, and optional model coordinates; capture cwd
  capture config.yaml and one non-creating connections.yaml snapshot for a new Session
  resolve invocation > stored preference > policy default
  load the exact Provider/Account credential when a stored model is selected
  install TerminationCoordinator
  open Host identity and Session repository
  normalize workspace and create SessionDescriptor
  spawn CodexBackend transport or assemble the dialect-derived native model backend
      ↓
yo-core AgentSession
  start worker
  attempt descriptor envelope
  CreateSession
      ├── CodexBackend → app-server initialize + thread/start
      └── NativeModelBackend → bind local exact-replay Session state
      ↓
yo-core
  SessionCreated
      ↓
yo-tui
  acquire terminal and enter Inline or Fullscreen mode
```

| Step | Current owner | What to follow |
|---|---|---|
| 1 | [`yo-cli/src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs), [`yo-cli/src/connection.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection.rs) | `run` selects presentation options, captures the working directory and command-local configuration, reads a new Session's stored preference without creating state, installs termination coordination, opens Host identity plus Session storage, canonicalizes the workspace, and creates one matching UUIDv7 `SessionDescriptor`. Resume omits the stored-preference read. |
| 2 | [`yo-cli/src/model.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/model.rs), [`yo-core/backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs), [`yo-core/backend/native`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/native/mod.rs) | The process host resolves invocation, stored, and operator layers, then either starts the Codex stdio transport or assembles the selected native binding from the startup snapshots and injected tools. Both defer remote model work until the worker owns the backend. |
| 3 | [`yo-core/agent_session`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/mod.rs) | `AgentSession::start_cancellable_with_repository` transfers the backend and local repository to the worker thread (named `yo-agent-runtime`) and waits for startup without blocking termination observation. |
| 4 | [`yo-core/agent_session/worker.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/worker.rs) | `AgentWorker::initialize` first attempts the descriptor-only Journal envelope, then sends `CreateSession` through `AgentRuntime`; storage pressure keeps both the descriptor and later activity in the recoverable volatile prefix. |
| 5 | [`yo-core/backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs), [`yo-core/backend/native`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/native/mod.rs) | For Codex, `CreateSession` performs `initialize` and `thread/start`. The native backend binds local exact-replay Session state without a provider request. Both let the semantic engine produce `SessionCreated`. |
| 6 | [`yo-tui/runner/unix.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/unix.rs) | `run_session_with_mode` acquires input and terminal state for the first terminal ownership generation, then enters the already selected presentation mode. |

If termination arrives during the handshake, `AgentSession::start_inner`
observes the cancellation callback, requests the backend stop, waits for worker
cleanup, and returns without giving the TUI a Session. Start investigation
there, not in terminal mode code.

The public host flags are `--inline` or `--fullscreen` for presentation and
`--ascii` for the built-in ASCII glyph profile; flags may appear in either
order. Omitting the presentation flag keeps the Inline default, and omitting
`--ascii` keeps the Rich compatibility default. Unknown flags, repeated
`--ascii`, and multiple presentation flags fail before provider or terminal
startup. The selected glyph profile constructs the retained `TuiSession`, so
prepared frames and final plain session output read the same committed
appearance snapshot. Glyph selection is explicit and does not inspect `TERM` or
`NO_COLOR`. Separately, the CLI classifies explicit `COLORTERM=truecolor|24bit`
as TrueColor, a `TERM` containing `256color` as Limited, and suppressed or missing
evidence as Unknown; only TrueColor enables the RGB activity ramp. The CLI also
passes its known backend name and a home-compacted working-directory label into that retained session. These labels are display
metadata only; they do not select or identify the backend Session.

Contracts:
[session publication](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.session-publication.md),
[frame consistency](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.frame-consistency.md),
[glyph profiles](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.glyph-profiles.md),
and
[resolved cell style](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.resolved-style.md).

## Workspace reference assistance

Typing an eligible `@query` in Chat follows a separate nonblocking route:

```text
PromptEditor + cursor
  ↓ revision-bound trigger snapshot
yo-core local execution workspace provider
  ↓ Git-ignore-aware files/directories + deterministic Unicode-normalized ranking
TuiState prompt overlay
  ↓ Tab or Enter
replace the exact @query span and retain its typed identity
```

`yo-tui` owns scanning, stale-result rejection, overlay input, and editor-span
transforms. `yo-core::LocalWorkspaceReferenceProvider` owns local execution
discovery semantics and performs Git and filesystem work on its worker thread;
`yo-cli` only constructs and connects that capability.
The candidate and request/update types live in `yo-core`, so a remote execution
provider can replace the local connection without moving filesystem authority
into the frontend. The inventory includes visible files and directories,
honors nested Git ignore, repository exclude, and configured global excludes,
and does not follow directory symlinks. Each row keeps its basename and dimmed
parent path together in the left reading flow and reserves the right edge for
the neutral `File` or `Dir` kind. Directory labels and accepted tokens end in
`/` so their kind remains visible while typing. The first query may show a
searching state in the header; continuous typing keeps the current panel until the newest
result arrives, then redraws once instead of flashing an intermediate loading
frame. The panel title is `Files`; its header derives hints from the active
bindings, emphasizing keys while dimming captions. Rich glyphs use `↑↓` for
movement, ASCII uses `Up/Down`, and familiar terminal names such as `Enter`,
`Esc`, and `^C` remain literal.

This Slice deliberately stops before structured submission admission. Selecting
a row visibly replaces the token and retains the typed reference, but a later
Enter preserves the draft and reports that structured submission is not yet
connected. It never silently degrades an accepted identity into plain text.

## Explicit skill assistance

Typing an eligible `$query` reuses the prompt trigger lifecycle but discovers
metadata through a separate frontend-neutral skill port:

```text
PromptEditor + cursor
  ↓ revision-bound $ trigger
CodexSkillReferenceProvider worker
  ↓ Codex skills/list descriptors for the current cwd
Skills overlay
  ↔ Left/Right filters cached rows by All, Workspace, User, System, or Admin
  ↓ Tab or Enter
replace the exact $query span and retain the catalog identity and revision selectors
```

The catalog worker owns a short-lived Codex app-server connection and never
blocks the terminal event loop. It uses only the `repo`, `user`, `system`, and
`admin` scopes reported by Codex; Yo does not infer provenance from filesystem
paths. Duplicate names remain separate identities, and disabled skills remain
visible with a reason but cannot be selected. The local adapter hashes the
exact `SKILL.md` bytes into the entry revision; an unreadable revision disables
that row instead of issuing a selector that admission cannot later verify. A
newly opened Skills overlay forces a fresh `skills/list` snapshot and advances
its catalog generation; continuous typing coalesces queries against that same
snapshot. The optional scope filter lives
only in the bottom-left panel footer. Left and Right operate on the already
received candidates, so changing the filter neither reruns discovery nor
reflows the prompt.

V1 retains at most one accepted explicit skill. Selection does not read the
skill body, execute it, inject it into model context, or submit the draft.
Until submission-time admission can reload and revalidate the exact selected
entry, Enter preserves the draft and fails closed rather than treating the
visible `$name` as sufficient authority.

## One active turn

A submitted prompt follows this route:

```text
terminal input
    ↓
TuiState::handle
    ↓ immutable InputSubmission
TuiAgentConnection
    ↓
AgentSession queueing and bounded command lane
    ↓
AgentWorker
    ↓ accept or reject the same SubmissionId
    ↓ AgentCommand::StartTurn or SteerTurn
AgentRuntime
    ├── validate with AgentEngine
    ├── accept through AgentBackend
    ├── commit with AgentEngine
    └── append command and events to SessionJournal
          ↓
Codex app-server adapter
    ↓ BackendEvent
AgentRuntime
    ↓ commit and append to SessionJournal
AgentSession coalescible change lane
    ↓ wake-up only
TuiAgentConnection + TranscriptReader + RequestTraceReader
    ↓ ordered AgentPoll::Record / RequestTrace
    ↓
TuiState::observe_record
    ├── concise Chat projection
    └── chronological Transcript / full-Session Request projections
          ↓ selected view
completed Surface
    ↓
Inline or Fullscreen presenter
```

The useful inspection points are:

1. [`TuiState::handle`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/state.rs)
   captures one immutable `InputSubmission`. Plain text stays visible until an
   `Accepted` outcome with the matching `SubmissionId` arrives. If the user has
   edited a newer draft meanwhile, that newer text is not cleared. A rejection
   preserves the draft, and duplicate or stale outcomes have no effect.
2. [`TuiAgentConnection`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/agent/mod.rs)
   is a narrow local adapter. It forwards dispatch, retry, and submission
   outcomes; turns
   a coalesced Session change notification into bounded `TranscriptReader`
   suffix reads, and exposes ordered records to the TUI. It owns no Session or
   provider semantics.
3. [`agent_session/admission.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/admission.rs)
   resolves Submit to `StartTurn` or `SteerTurn`. `Queued` means only that the
   bounded worker lane now owns the command; it is not final acceptance. A busy
   state lock or full lane returns an opaque pending command carrying the same
   `SubmissionId` for the TUI loop to retry. The first dispatch reserves that ID
   for the Session; reusing it is rejected before another backend command can run.
4. [`AgentWorker`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/worker.rs)
   is the only owner that executes and polls the runtime. After runtime and
   backend acceptance succeed, it publishes `SubmissionOutcome::Accepted` for
   the exact ID. The typed rejection channel exists for the next reference-
   admission Slice; structured `@` and `$` drafts remain fail-closed until then.
   The terminal-owning thread does not wait on provider I/O.
5. [`AgentRuntime`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/runtime/mod.rs)
   orders command validation, backend acceptance, semantic commit, and Journal
   publication. `StartTurn` and `SteerTurn` enter through the correlated
   submission boundary; the ordinary command boundary rejects them without a
   `SubmissionId`. The worker-owned durable writer maps text updates to bounded
   immutable segments and synchronously appends a semantic commit before its
   committed record is exposed. Authoritative backend snapshots start a new
   message revision instead of mutating an already durable segment. Consecutive
   replacements that have emitted no segment share that unpublished revision;
   an empty replacement is represented by `MessageReset` when a time or ordering
   boundary makes it durable, or by its zero-byte terminal seal at termination. It also
   translates provider observations through the semantic engine before
   publishing a change notification. Rejected commands and invalid backend events are not
   recorded as committed semantics; terminal events created while closing a
   failure are.
   `AgentSession::transcript_reader` exposes bounded, read-only suffix copies
   from that same Journal without exposing its lock or storage layout.
   Capacity or storage failure publishes the semantic result as a volatile live
   suffix and latches `JournalDurability::Gap`. Once storage accepts writes
   again and every open message has a real terminal seal, the same writer
   publishes one complete snapshot before returning to incremental commits.
   Empty messages still receive a zero-byte terminal seal, and recovery derives
   an interrupted zero-byte seal when a crash follows `ActivityStarted` before
   the first text segment. Observable plan or reasoning-summary text admitted
   by an adapter as semantic `ModelWork` uses the same segment and seal path.
   Hidden reasoning yo never receives and unadmitted backend-specific Request
   Audit payloads remain outside this semantic path. The shared observation stream orders every typed
   durability transition before the semantic records affected by it, so a
   coalesced worker wake-up cannot erase a Gap-to-Durable transition. The same
   level-triggered readiness also wakes the terminal owner instead of waiting
   for a periodic agent poll. The CLI
   adapter forwards that order to TUI state with the exact cutoff class. Chat, status-row, or banner presentation
   remains a separate product contract. Stored-Session inspection follows the
   separate read-only path below. Executable continuation uses the separately
   validated recovery path below rather than deriving state from that frontend
   history projection.
6. [`runner` source scheduling and redraw](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/unix.rs)
   use a rotating cursor to select one ready terminal, agent, workspace, or
   skill observation at a time. The selected observation updates TUI state;
   process termination remains a strict-priority path outside that rotation.
   The runner composes a completed `Surface` and sends it to the active presenter. `runner/view.rs`
   selects Chat, Transcript, or Request from the same record stream. Chat shows
   user input only when its `StartTurn` or `SteerTurn` command appears in that
   sequence. Terminal `EventStream` readiness and the agent, workspace, and
   skill producer readiness wake the owner thread. Their live-source traits
   require this contract; there is no periodic observation fallback. Unix
   termination handlers publish the durable signal bit and perform only a
   nonblocking, async-signal-safe write. A normal notifier thread converts that
   byte into the same frontend wake before the host cleans up and replays the
   selected original signal. State changes request a frame
   instead of drawing synchronously for every event. `FrameScheduler` publishes
   the first and resize frames immediately, then coalesces ordinary requests at
   the `TuiSession` limit: 120fps by default, or 60fps when the host selects
   `FrameRateLimit::Fps60`. With no readiness or scheduled frame, motion, or
   active-backpressure deadline, the owner may sleep indefinitely. The 10ms
   backpressure retry remains a deadline only while an operation is actively
   retained. An editor mutation that dispatches
   `@` or `$` discovery requests a frame before any provider result; the prior usable panel remains visible
   behind a pending snapshot gate. A Chat frame that actually paints the elapsed-selected
   Rich Braille or ASCII work-marker frame inside its fixed maximum-width region, or a
   fixed-text activity sheen, returns the shortest visible period;
   the runner schedules the next generation-epoch boundary and coalesces it with
   event redraw. Hidden, narrow, short, idle, reduced-motion, and zero-size indicators
   do not arm that timer. A one-grapheme activity status can still pulse and therefore
   remains animated.

   Inline Chat preparation is a two-part transaction. `runner/state.rs` selects
   the maximal contiguous prefix of complete unpublished items as persistent
   output, then composes only the remaining transcript suffix, prompt, chrome,
   and overlay into a natural-height live `Surface`. `terminal/mode/inline`
   compiles the persistent rows and live update into retained typed
   `TerminalOp` groups before the shared ANSI encoder and direct unbuffered Unix
   transport. The effect ledger carries the observed terminal geometry plus
   cursor ranges and distinguishes an addressable prefix, definite scrolling,
   and a possible-scroll state whose anchor is not exact. Exact downstream
   progress permits one bounded clear-and-restart or suffix-resume recovery at a
   complete operation boundary; a partial operation, possible scroll, or second
   failure is fatal. A recovered correction is retained as bounded `TuiSession`
   environmental evidence. The publication cursor advances only after that
   write and flush complete. The presenter then drains queued resize notifications and
   samples terminal size: it keeps the persistent acknowledgement but rejects
   the prepared live geometry whenever either the size or geometry epoch is
   stale, and requests an immediate suffix-only reprepare. A detached Chat
   viewport, Transcript, and Request freeze publication; Fullscreen does not use
   the cursor.

The accepted ordering, interruption gestures, honest status data, and
responsive fitting policy are owned by the
[static input chrome contract](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.chrome.input-stack.md).
In this runtime, `shell::chrome` calculates and fits typed rows from active
state and `TuiSessionInfo`, and `shell::chrome::help` removes complete
low-priority actions rather than wrapping or truncating their labels. The
shared `input::key_notation` formatter renders terminal conventions such as
`Esc`, `^C`, `^D`, and `S-Enter` from the configured semantic bindings; it does
not decide whether an action is currently available. `shell` composes those
regions around the prompt, and `input::control` dispatches the mapped interrupt
intent even when a tiny frame cannot show the visual hint.
The 80ms marker-frame sequence, maximum-width marker region, continuous two-second
shimmer, and the configurable 120/60fps runner frame boundary are owned by the
[activity motion profile](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.activity-motion-profile.md)
and
[activity motion scheduling](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.activity-motion-scheduling.md)
contracts.

The change lane carries no command or event payload and has capacity one.
Multiple commits may therefore share one unread wake-up without losing
history: the concrete local reader continues by Journal sequence until it
reaches the observed head. A terminal backend failure is reported only after
the adapter has exposed the failure records already committed to the Journal.

Codex JSON and provider identifiers end at the backend adapter. Terminal input
events and rendering types end in `yo-tui`. The command and event types crossing
the middle are owned by `yo-core`.

## Stored Session inspection

Stored history never enters the live startup path:

```text
yo session [--all] [--details]
  ↓ read existing Host identity and repository without creating either
LocalSessionReader::discover
  ↓ validated tail summaries
workspace-filtered metadata table on stdout

yo session SESSION_ID [--view chat|transcript|request]
  ↓ one point-in-time physical snapshot, no writer lease
yo-core read_stored_session
  ↓ envelope validation + Journal recovery
StoredSessionHistory
  ↓ Chat/Transcript message normalization or full-session Request correlation trace
  ↓ exact Journal boundary; Request Audit explicitly unavailable without a reader
yo-tui archived projection
plain stdout
```

`yo-cli/src/command.rs` owns the command grammar, `session.rs` owns selection
and table/output routing, `config.rs` owns date-format configuration, and
`storage.rs::open_default_reader` is deliberately separate from writer startup.
Request has no anchor selector: it renders every durable correlation and
availability record in chronological Journal order instead of guessing a
nearby request. The projection never prints backend payloads or physical
repository envelopes.

For terminal stdout, `session.rs` supplies the observed width and Session-specific
column priorities and continuation hints to the generic `yo-tui::plain`
renderer. It first folds PATH and DETAIL, then continuation/version, started
time, and workspace. Short folded label/value pairs flow left to right below
their primary row and move as complete pairs when the next one cannot fit.
PATH and DETAIL flush that flow and take an independent row. Their label and
value stay inline when the pair fits, and split into a wrapped labeled block
only when needed; an oversized flow pair is promoted to that wrapped block
form. Records with folded
details are separated by exactly one blank line. If the pinned identity,
status, and updated time still cannot fit, every field becomes a labeled
vertical card and the shared table header disappears. Folded values wrap at
terminal grapheme-cell boundaries and are never truncated. If a single atomic
grapheme is wider than the entire reported terminal, rendering fails explicitly
instead of splitting or dropping it. Terminal headings start at the left edge
in bold, with only their values indented by two cells. Non-terminal stdout
uses an unbounded one-line table so pipes and redirected files do not depend on
the invoking terminal's width and never contains ANSI styling.

The optional configuration file is read but never created. Linux uses
`${XDG_CONFIG_HOME:-$HOME/.config}/yo/config.yaml`; macOS uses
`$HOME/Library/Application Support/yo/config.yaml`. `YO_CONFIG` selects an
explicit path. The current pre-version schema is:

```yaml
session:
  list:
    date_format: "%Y-%m-%d %H:%M %:z"
tui:
  max_fps: 120
```

`config.yaml` owns only general Session and TUI settings. A top-level `model`
field is unknown. Model definitions, catalog seeds, and the startup preference
have one durable owner: `connections.yaml`.

`yo connect --from /absolute/definition.yaml` reads one transient grouped
definition. Exact `--from -` reads the same shape from standard input. The
document names one Provider and Account, one endpoint, a required base-profile
mapping, and 1 to 4,096 models. The mapping may be partial or empty. A model may
replace any closed profile field as a whole; omitted fields inherit the base
value, structured mappings are not recursively merged, and every resolved
model profile must be complete. For example:

```yaml
provider: example
provider_display_name: Example
account: team
account_display_name: Team
base_url: https://api.example.test/v1
profile:
  api_dialect: openai-responses
  tokenizer_profile: utf8-bytes/v1
  input_token_limit: 1000000
  max_output_tokens: 65536
  reasoning_parameters:
    effort: medium
  optional_request_parameters: {}
  tool_capability_policy: local-tools/v1
models:
  - model: model-a
    model_display_name: Model A
  - model: model-b
    profile:
      api_dialect: openai-chat-completions
      max_output_tokens: 8192
```

The import replaces that Provider-and-Account group atomically; omitted old
models are removed. It never chooses a default. An existing preference is
preserved unless it names a model removed by the replacement. One preview, one
confirmation, and one credential capture cover the whole group. The preview
compares the complete account metadata and exact catalog or discovery seed,
names added, changed, and removed models, and warns that saved Sessions using a
changed or removed complete binding may not resume until that exact binding is
restored. The non-interactive form requires either `--from PATH` with an
absolute `PATH` or exact `--from -`, plus an absolute `--credential-file PATH`
and `--yes`; neither YAML document may contain the secret.

A release-known QwenCloud or Kimi catalog uses `catalog` instead of
`base_url`, `profile`, and `models`:

```yaml
provider: qwencloud
provider_display_name: QwenCloud
account: team
account_display_name: Coding Team
catalog: qwencloud-coding-plan-intl/v1
```

The closed QwenCloud identifiers are `qwencloud-coding-plan-cn/v1`,
`qwencloud-coding-plan-intl/v1`, and
`qwencloud-token-plan-team-intl/v1`. Kimi accepts
`kimi-platform-ai/v1` and `kimi-code-membership/v1`. A stored seed creates
connect candidates, not a startup-routable binding. Selecting one candidate
stores its complete profile, including explicit private-replay consent where
required; catalog identity or ModelId alone cannot manufacture that consent.

OpenRouter discovery uses the explicit shape but omits `models`; it is the only
Provider allowed to do so. Its stored seed supplies the endpoint and base
profile for the bounded authenticated picker.

The date syntax is strftime-compatible and both UPDATED and STARTED are shown
in the viewing machine's local timezone. `tui.max_fps` accepts numeric `60` or
`120`; live startup reads it once and applies it to retained TUI generations.
Runtime reload is not supported. Whole-field YAML null, unknown or duplicate
fields, duplicate ModelIds, incomplete profiles, and a relative `--from` path
fail before credential capture or mutation. `{}` is an explicit empty
structured replacement; nested null remains a structured value. Plain YAML 1.1
`y`/`yes`/`true`/`on` and `n`/`no`/`false`/`off` spellings are
case-insensitive booleans, and `1_000` is integer `1000`; quoted forms remain
strings. The producer persists complete profiles so startup and native resume
do not repeat authored inheritance.
Unreadable files, retired fields, unknown fields, oversized files,
invalid date formats, and unsupported frame rates are explicit failures rather than silent
fallbacks. The reader opens one no-follow nonblocking descriptor, requires it to be a
regular file, captures stable identity and metadata, and consumes at most 1 MiB plus one sentinel
byte, so a FIFO cannot stall the command and concurrent file growth cannot bypass the bound.
Preference mutations recapture this file and require the exact command-local snapshot to remain
unchanged before public commit. A
model API key is never read from an environment variable. When a configured
model is selected, Yo reads a separate `credentials.yaml` beside the selected
`config.yaml`. Its current pre-version Provider-then-Account shape is:

```yaml
providers:
  openrouter:
    default:
      api_key: "..."
  qwencloud:
    default:
      api_key: "..."
```

The credential file must be a current-user-owned regular file with no group or
other permission bits (normally mode `0600`). The same Account ID may be used
under different Providers; only the exact selected Provider-and-Account pair is
resolved. A revision-less current-shape file remains a readable snapshot.
`LocalCredentialRepository` re-reads it under a private store lock and can
prepare exactly one add, replace, or remove without retaining the candidate
secret. Commit accepts the candidate only for add or replace, preserves every
unrelated pair, and atomically publishes a complete mode-`0600` snapshot with
an independently generated private `crev-...` receipt. The planned receipt and
exact pair state make a repeated commit idempotent, while a different observed
revision is a conflict. Removing the final pair leaves a revisioned empty file
rather than returning to `absent`. These writes are a core storage boundary.

`config.yaml`, `credentials.yaml`, `connections.yaml`, and the operation
journal share `yo-yaml`: exactly one document, finite structural and replay
budgets, bounded small aliases, and rejection of duplicate keys, merge keys,
unknown aliases, cycles, and additional documents. None carries a top-level
format-version field. An unknown `version` field or journal `profile_digests`
field is an ordinary unknown field and fails typed decoding before mutation.
Yo does not classify, decode, migrate, dual-write, downgrade, or automatically
delete an older pre-release shape.

The sibling `connection-operation.yaml` owns the secret-free durable intent
for a credential-and-public operation. The current pre-version record carries
an opaque operation ID, exact expected and planned public revisions plus the complete bounded prospective
public snapshot, one add, replace, remove, or preserve credential receipt, and
one legal phase.
It cannot accept an `ApiCredential`, candidate identity, or
verification payload. Missing capture is non-creating; first intent publication
is exclusive; every phase replacement is bounded, mode `0600`, no-follow,
current-user-owned, durable, atomic, and exact-entry checked.
Every journal mutation requires a mutable `LocalConnectionOperationGuard` for
the same repository directory. The guard's nonblocking file lock excludes a
second process-equivalent owner across the capture-and-publication boundary;
the mutable borrow prevents concurrent mutation calls from sharing one guard,
and a guard from another directory fails before any journal bytes change.

`plan_connection_recovery` is a pure state-table boundary. Connect abandons an
uncommitted expected/expected intent, resumes only the public CAS after the
credential reaches its exact planned receipt, and completes only on the exact
planned public bytes. Disconnect abandons expected/expected, commits a prepared
remove only after the public snapshot is exact planned, or preserves the exact
credential revision without mutation. A phase ahead of repository facts, a
credential-first disconnect, a different public winner, or any unlisted state
is a typed conflict without exposing a private credential revision.
`LocalConnectionOperationRepositories` admits only absolute normalized paths
with the three closed sibling filenames in one lexical directory and rejects a
symbolic-link component before acquiring the shared operation lock. After
creating a missing state directory with user-only mode, it
captures the directory's device and inode before acquiring the lock and verifies
the same pathname identity immediately after acquisition,
then checks the pathname components and identity again before each journal or
repository capture and effect. A directory replacement or symbolic-link
retarget in those checked gaps therefore fails before the next mutation. This
is a fail-closed pathname revalidation boundary; it does not claim protection
against an adversarial ABA replacement or provide an atomic directory-descriptor
anchor across a check and following filesystem call. The session executes
only the state-table decision: it abandons an uncommitted intent, catches a
lagging phase up before and after the exact next repository CAS, or advances an
already completed repository pair through `complete` and clears the exact
journal. Connect recovery never reconstructs or commits a secret; disconnect
removal passes no candidate, while preserve never calls the credential mutation boundary.
Repository and journal failures retain the safe operation kind, action, and
phase without projecting a private credential revision. External connect now
uses this same held session for preparation and commit. External disconnect
uses it to bind one selected stored target to the same Provider-and-Account
credential action and commits the public removal before any credential removal.

`yo connect qwencloud:Account` resolves that Account's stored QwenCloud
catalog seed from `connections.yaml` and opens the same controlling-TTY picker before reading a
credential. Every release-known row remains visible; a row unsupported by
Yo is disabled with its reason. Cancellation or disabled selection performs
no credential read and creates no intent or repository mutation. An exact
`yo connect qwencloud:Account:Model` bypasses the picker, while a Model outside
the selected closed catalog fails with guidance to replace the stored definition
through `yo connect --from`. There is no remote model-list request: after one selectable row is
chosen, structural binding admission, preview, credential capture, journal,
and commit path remains authoritative. Registration itself sends no model
request and does not claim that the account can use the selected row.

`yo connect kimi:Account` reads one candidate key and fetches one bounded
authenticated `GET models` snapshot from the stored Kimi product seed,
then passes the normalized typed rows to the same picker. The first valid exact
ModelId wins; more than 4,096 rows reject the whole snapshot. Platform admits
only its reviewed K3, K2.7 Code, K2.7 Code Highspeed, and K2.6 envelopes. Code
Membership admits exact `k3`, `k3-256k`, `kimi-for-coding`, and
`kimi-for-coding-highspeed` envelopes; `k3-256k` is recommended. Cross-product
and future rows stay visible with a stable disabled reason instead of being
hidden. Each row becomes selectable only when its remote context and reasoning
evidence stay inside that product's reviewed envelope. Before a K3/K2.7
stored binding is published, the compact preview states that bounded Kimi
assistant state will be retained unencrypted in current-user local Session
records.

Secret-free connection preparation retains a closed Kimi catalog/profile
compatibility check so invalid cross-product or limit rows fail before credential
or public-state mutation. This check neither builds Kimi wire values nor replaces
the Connector's independent pre-client validation.

The flat `yo-connector-kimi` crate then owns the exact request, stream,
provider-private assistant codec, visible projection, and encoded-size grammar
for that selected complete binding. Platform keeps its established
request shape. Code K3 sends its admitted reasoning effort with
preserved-thinking `keep: all`; Code K2.7 sends forced preserved thinking.
Both Code families also send one opaque `prompt_cache_key`. The backend creates
that hint once from the Session identity and reuses it across ordinary and
resumed requests without branching on Provider; the Connector alone decides
whether to serialize it. Hints are
redacted and never become binding identity, replay evidence, logs, diagnostics,
transcripts, or traces. Successful K3/K2.7 rounds emit one
bounded opaque provider-private envelope whose Kimi payload contains the complete
reasoning, content, and tool-call message. Core never interprets that payload. It is hidden from frontend and Request-trace
projection, stored atomically beside its visible assistant/function
projection, and admitted only after the completed neutral projection, exact private replay
profile schema, and binding epoch match. Its physical Journal member order and profile string
remain unchanged. The managed loop requires exactly one envelope after every completed
private-profile assistant-and-calls group, and recovery verifies the same ordering before
reconstructing a Continuation Anchor. The next Kimi request replaces that visible group with the
private assistant message exactly once. Semantic-only bindings cannot store or
replay the private item, and an incomplete or failed round creates no private
Continuation Anchor.

`yo connect openrouter:Account` is an interactive discovery target only when
that exact stored seed has a normalized endpoint and complete base profile.
After recovery and snapshot capture, Yo reads one no-echo candidate
key and issues an authenticated `GET` to the endpoint prefix plus
`/models/user`. The request has bounded same-origin redirects and separate
connect, per-attempt response-header, body-inactivity, and absolute deadlines;
success must be bounded JSON. Core normalization keeps the first row with a
valid exact Model ID even when its capabilities or profile make it unavailable.
It treats capability arrays as duplicate-free, order-insensitive sets, requires
text input and output, and narrows a configured local-tools policy to no-tools
when either remote `tools` or `tool_choice` capability is absent. Valid remote
context limits replace only fields without an authored model override. An exact
priority selects one typed disabled reason, while enabled rows alone carry a
selectable complete binding. The controlling-TTY picker searches name and ID,
exposes at most eight scrolling rows while keeping every match—including
disabled rows with their reason—reachable, blocks disabled selection, and
restores terminal mode, cursor, and dynamic panel on
selection, cancellation, input/render failure, or unwind. Remote strings cross
a printable reversible byte-escape boundary before terminal output. Search
editing removes one extended grapheme cluster per backspace, and wrapping and
clipping use terminal-cell width rather than byte or scalar counts. The bounded
raw-key decoder consumes complete CSI or SS3 sequences, maps plain and modified
Up/Down keys, and leaves no unsupported, malformed, or overlong sequence tail
as search text. A partial escape or UTF-8 scalar has a finite read deadline;
when an invalid UTF-8 continuation is itself an independent key, that byte is
preserved for the next decode. Selection then enters the existing concise
connection preview; `--verbose` expands only
that preview. Cancellation creates no new intent or repository mutation, and
the same in-memory key used for discovery is retained for publication after
final structural admission. Two-part discovery rejects `--credential-file`
and `--yes`.

`yo connect Provider:Account:Model` accepts one exact reference from the
captured stored definitions or a reviewed stored catalog seed. It forms the
prospective post-mutation set from every stored sibling for that Provider and
Account plus the selected complete binding. The old binding at the selected
coordinate is excluded from registration accounting; verbose preview may retain
it only to compare old and new profiles. The prospective stored upsert must pass
startup-policy admission before any secret is read. Yo
requires confirmation, then reads one bounded API key only from the controlling
TTY. Credential capture retains `ISIG`, clears `ECHO` and `ICANON`, sets
`VMIN=1` and `VTIME=0`, and restores the exact original terminal settings. If
explicit restoration reports an error, the retained guard retries restoration
while unwinding. Every line-oriented controlling-TTY prompt has a
16,384-byte input limit. On overflow Yo flushes the unread terminal input queue
before returning the limit error, reports a flush failure separately, and for a
credential prompt restores echo before returning either error. An external exact
target may instead use
`--credential-file PATH --yes`; both options are required together, `--yes`
conflicts with the interactive `--verbose` view, and Local Codex rejects the
pair before opening the file. After recovery and exact-plan preparation, this
path suppresses confirmation and opens the final credential path once with
no-follow semantics. It accepts only a current-user-owned regular file whose
mode is exactly `0400` or `0600`, reads through EOF under a 16,386-byte stable
metadata bound, removes at most one final LF or CRLF, and then applies the
16,384-byte UTF-8 `ApiCredential` rules. Capture failure creates
no new intent or repository mutation, does not fall back to the TTY, and never
changes or exposes the source file. Recovery may already have completed an
older operation before the new plan. Environment variables, secret argument
values, standard input, child processes, and config files are not credential
channels.
The confirmation first renders the complete preview in memory. Immediately
before publishing that preview and its prompt, Yo flushes queued controlling-TTY
input so only a fresh following line can authorize the plan; flush failure is a
distinct fatal input-boundary error before prompt publication or repository
mutation. Noninteractive `--yes` remains a separate captured-plan authorization
path. The confirmation presents the selected target, then uses stable semantic plan
markers (`+`, `~`, `−`, and `=`) to distinguish create, change, remove, and keep
effects. The default view keeps that decision-facing change set, names the
Provider and Account once on the credential action, lists each exact Model ID
registered for that account once, and ends with a concise plan count. `-v` or
`--verbose` groups models whose non-model connection and resolved
profile fields are exactly equal, then prints their shared non-secret endpoint,
dialect, and profile fields once. Any field difference creates a separate
profile group, so compaction never hides a distinct binding behavior. The
usual Model IDs remain bare; an ID containing a list delimiter or ambiguous
whitespace/quoting character uses reversible JSON-string quoting. When an item
and its separator cannot fit the inline list width, the list becomes distinct
bullet rows rather than splitting an otherwise fitting ID or orphaning a
separator. The
credential row is derived
from the prepared repository action, so adding a new key and replacing an
existing key cannot share misleading copy. A checked success summary closes the
command. Preview rendering uses the controlling-TTY width. Success rendering
samples standard output once: terminal output uses its nonzero column count,
while unavailable or zero width falls back to 80; redirected output is plain and
deterministic, and `NO_COLOR` keeps terminal output plain. Both paths wrap
terminal-safe nonzero-width graphemes without truncation or splitting and
preserve exact non-secret value bytes rather than relying on the shell's
incidental line wrapping. A two-cell atomic grapheme at width one fails with the
typed width error. The complete success output is prepared before the first
operation commit, so presentation failure cannot turn committed state into an
apparent command failure.

The command does not use the candidate key for a model request. After
confirmation it revalidates the captured config, publishes a
secret-free intent, commits the exact add or replacement credential, advances
the journal, publishes the exact stored public snapshot, advances to complete,
and clears the journal. Authentication, entitlement, and request acceptance are
learned only from ordinary model use. A crash after credential commit resumes
only the stored public bytes and never reconstructs or exercises a secret.

`yo disconnect` interactively infers a unique stored target or asks for one
exact captured `Provider:Account:Model` reference. Automatic execution requires
`yo disconnect PROVIDER --account ACCOUNT --yes` and proceeds only when that
pair has exactly one stored model; `--yes` never guesses among multiple
models. Before confirmation, Yo derives the prospective stored removal from
the same captured snapshot. The compact default preview uses the same semantic
plan markers for the stored removal,
default and API-key changes, and new- versus saved-Session effects. Its API-key
row names every remaining Model ID that still depends on that key within the
already named Provider-and-Account context, using the same reversible quoting
for an ambiguous ID. `-v` or
`--verbose` also shows the exact removed complete binding, source, and remaining
bindings for the pair. The preview resolves the prospective
startup layers and names the exact lower-priority target for new Sessions, or
states that no target remains; it does not infer that behavior from preference
removal alone. Remaining account models are exact Model IDs in that explicit
account context without repeating the removed profile. The same
controlling-TTY width boundary keeps every preview row within the observed width.
The checked success target and verbose remaining-model bullets pass through the
same reversible remote-text and ambiguous-item display boundaries as the preview.
Any remaining model or catalog seed for the same pair preserves the credential.
Only an empty post-removal dependent set prepares credential removal; an absent
credential fails before intent rather than inventing state.
After confirmation and the final config guard, the command publishes the
secret-free intent, commits the public removal, advances `public_committed`,
optionally removes the credential, advances to `complete`, and clears the
journal. Existing Session history is not deleted, but a Session attributed to
the removed complete binding may no longer resume natively unless the exact
binding is restored; the preview states
that continuation result separately from stored-history preservation.

Endpoint, model, API dialect, derived connector identity, the resolved profile, and display
names remain non-secret binding data rather than secret-file content. Catalog limits and model
IDs are operator-owned examples and must be checked against the exact current
Provider offering. `utf8-bytes/v1` conservatively counts the complete serialized
request one token per UTF-8 byte; `o200k_base/v1` is available only for bindings
whose tokenizer is actually o200k-compatible. Unknown profiles fail startup.
`max_output_tokens` is an optional known profile hard maximum. Producers omit it
when unknown, whole-field `null` is invalid, and absence survives base/model
resolution and durable complete-binding identity. Each known-cap round chooses
a positive request-local value no greater than that hard maximum and admits only
an exactly recounted connector payload whose input plus cap fits the input
limit. An unknown-cap round omits the dialect output field and requires its
exact input count to remain strictly below the input limit; connectors whose
wire contract requires a known cap, including the closed Kimi profiles, reject
unknown. The first explicit
runtime supports an empty reasoning mapping or an `effort` of `none`,
`minimal`, `medium`, or `high`; it requires empty
`optional_request_parameters` and `local-tools/v1`. Other validated profile identifiers remain readable
configuration but fail startup until their runtime behavior exists.

The public sibling `connections.yaml` is separate from general `config.yaml`
and secret `credentials.yaml`. It is the sole owner of stored accounts, complete
model profiles, catalog or discovery seeds, and the selection-owned preference.
A representative snapshot is below (the opaque revision value is illustrative):

```yaml
revision: rev-0123456789abcdef0123456789abcdef
preference:
  kind: model
  provider: qwencloud
  account: default
  model: qwen3.8-max
bindings:
  - provider: qwencloud
    account: default
    model: qwen3.8-max
    model_display_name: Qwen 3.8 Max
    connector: openai-responses
    base_url: https://example.test/v1
    profile:
      api_dialect: openai-responses
      tokenizer_profile: utf8-bytes/v1
      input_token_limit: 262144
      max_output_tokens: 8192
      reasoning_parameters: { effort: medium }
      optional_request_parameters: {}
      tool_capability_policy: local-tools/v1
    last_failure:
      kind: rate_limited
      observed_at: 2026-08-17T09:10:11Z
accounts:
  - provider: qwencloud
    provider_display_name: QwenCloud
    account: default
    account_display_name: Default
catalogs:
  - kind: built_in
    provider: qwencloud
    account: default
    catalog: qwencloud-token-plan-team-intl/v1
```

`last_failure` is optional warning-only observation state, not part of complete
binding identity and not a routing prohibition. Actual native model use reports
one closed typed outcome without retaining a secret, request body, response
body, or raw Provider error. The stored failure contains only `kind` and a
canonical UTC whole-second `observed_at`; the next successful model request
removes it. Authentication, authorization, exact-model availability,
rate-limit, other request rejection, Provider availability, transport,
timeout, protocol, configured response-limit, and local binding or credential
prerequisite failures have distinct kinds. User cancellation, local-tool
failure, and cleanup failure do not create an observation.

The request retains the exact complete binding and private credential revision
it used. After the request finishes, the connection owner briefly enters the
same operation lane, recovers a pending connection operation, and re-reads both
repositories. It publishes one `connections.yaml` CAS only if that binding and
credential revision are still current. A removed or replaced binding or a key
rotation therefore discards the stale outcome. Observation persistence failure
is reported separately and never changes the underlying request outcome. A
captured failure is shown as a warning in a later model-picker snapshot but
does not disable, hide, or deprioritize the row.

An absent file is the canonical unset snapshot and is read without creating a
directory. Capture rejects unknown fields, duplicate account or binding
coordinates, bindings without their account, inconsistent Provider display
metadata, invalid complete bindings, and out-of-range unquoted structured-profile
numbers. Whole-field null is rejected even for optional fields; nested null and
quoted numeric-looking strings retain their exact structured variants.

An exact-target connect adds or replaces one complete model and preserves its
stored siblings. A grouped import replaces one entire Provider-and-Account
definition, including its catalog seed, in one revision. Stored removal removes
one exact model, retains the account and credential while a sibling or seed
still uses the pair, and clears only an exact matching ModelTarget preference.
Preference-only preparation preserves every stored definition. All mutations
reserve one new opaque revision and use the existing old-or-exact-new CAS. An
absent first write uses same-directory exclusive publication; later writes use
durable atomic replacement. Exact planned revision and bytes are idempotent
success, while another revision is a conflict. Credential-changing connect or
import reserves a new public revision even when the visible definition is
otherwise equal, giving recovery an exact public epoch for key rotation.

Every live startup captures `config.yaml` and one `connections.yaml` snapshot.
The snapshot directly supplies the model catalog and preference; there is no
manual/stored composition or provenance conflict path. Startup, resume matching,
and the live model picker use the same complete stored profiles.

`yo default TARGET`, `yo default --unset`, explicit `yo connect host:codex`,
external model connect, and external model disconnect use one nonblocking
process operation lock and resolve pending multi-repository work before reading
new command configuration. The
preference-only commands publish one public CAS after target admission or Local
Codex verification plus the final configuration guard; they do not create a new
operation journal or inspect credential revisions, and re-encoding preserves
stored definitions. External connect, import, and disconnect use their operation-specific
journaled sequences above. Free-form Provider onboarding remains unimplemented
rather than borrowing a weaker path.

A missing repository produces an empty list and does not create state. Direct
history reads preserve a message-recovery
interruption in the semantic records and send discovery disagreement
diagnostics to stderr. The physical `v1` format cannot prove whether a stopped
writer had an unpersisted volatile suffix, so stored history records durability
continuity as `not-observable` instead of claiming completeness. The Chat
projection remains concise and pipeable while its default direct command emits
that continuity boundary on stderr. Transcript adds the captured Journal cutoff,
message-recovery state, durability-continuity boundary, discovery consistency,
and chronological semantic records. Missing and present-but-incomplete physical
histories remain distinct direct-read failures. Neither archived form starts a
backend, follows later appends, repairs storage, or itself offers continuation.
The live `yo --resume UUID` and `yo --continue` paths instead use the dedicated
typed continuation recovery described below.

## Live observation views

The selected TUI projection changes presentation, not Session authority:

```text
read-only AgentPoll stream
    ├── Chat: concise activity/message projection + editable prompt
    └── full semantic record projection
          ├── Transcript: chronological command/event and Activity detail
          └── RequestTrace: full-Session correlation records in Journal order
                ├── exact Chat/Transcript context → optional highlight only
                └── Request Audit → explicitly unavailable
```

F1/F2/F3 currently select Chat/Transcript/Request through
`input/view_binding.rs`. That mapping is a typed presentation-policy seam, not
projection state. Page and line navigation update the active view's own
viewport; Chat and Transcript also retain their own context cursor. Request
navigation scrolls the complete diagnostic trace and never changes its content
by selecting a nearby request. Returning to a view restores its retained state.

All three modes use the session's pinned appearance snapshot and the existing
Transcript layout and Surface primitives. The status row shows the active mode
and keys, switches to a compact `[C]123`, `[T]123`, or `[R]123` form on narrow
frames, and remains renderable when only one terminal row is available.
Transcript and Request are full-page read-only modes: their input path never
reaches the prompt editor or emits a submission.

The TUI adapter exposes semantic `TranscriptRecord` values, typed durability
transitions, and a separately paged payload-free `RequestTraceEntry` stream.
The Request stream preserves each correlation record's `JournalSequence` and
is drained from the same worker change notification without exposing Journal
locks, backend payloads, or physical storage. Request Audit detail remains
explicitly unavailable. An exact `ActivityRequestRef` from the currently viewed
Chat or Transcript record may be shown as context, but it does not alter the
full trace.

## Durable Journal composition seam

The live `AgentSession` uses this local composition:

```text
initial SessionDescriptor (replay sequence 1, no semantic cutoff)
    ↓
semantic Journal records with explicit JournalSequence
    ↓ runtime adds binding, accepted-request, outcome, and Anchor correlation
    ↓ codec/recovery validates the complete correlation graph
    ↓ bounded MessageSegment construction
JournalCommit codec
    ↓ one semantic commit
JournalRepository
    ↓ validate with durable semantic prefix
    ↓ one physical append
SessionRepository
    ↓ add writer timestamp; checksum payload + complete discovery summary
Session-single-writer versioned JSONL physical v1

versioned JSONL
    ↓ bounded suffix read + semantic decode
Journal recovery
    ↓
RecoveredJournal or an explicit recovery error
    ↓ derive binding epoch and latest complete Anchor per physical commit

existing repository root
    ↓ LocalSessionReader (no create, repair, or writer lease)
last complete envelope of each Session
    ↓ validate closed v1 shape + CRC32C
available discovery summary or typed per-Session unavailability
```

The reader classifies quarantine, corruption, unsupported schema, and a missing
complete envelope without parsing diagnostic strings. A supported summary with
no Continuation Anchor is `unavailable`; an unsupported schema is `unknown`.
It probes an observed pending marker against both the exact Session lease and
the marker inode's independent append lock without creating storage. Only a
live owner holding that exact marker lock exposes the pre-append cutoff; a
successor cannot adopt an inherited marker, which quarantines that Session
without hiding other Sessions. If another append replaces the marker pathname
while it is being inspected, the reader detects the inode-generation change and
retries against the replacement within a fixed bound instead of reporting a
false quarantine.

A writer-capable repository retains a shared guard on the legacy root lock for
mixed-version safety. Before loading or repairing a Session it acquires that
Session's exclusive writer lease and retains it for its lifetime. The final
repository-wide capacity check, marker publication, append, synchronization,
rollback, and marker removal run under a short-lived root append coordinator;
the coordinator is released between appends and its files do not consume record
capacity.

Before the backend receives `CreateSession`, the worker attempts one
descriptor-only incremental envelope containing the UUIDv7 Session identity,
Workspace Host identity, the producing Host's canonical path bytes, and the
matching start time. The descriptor is Journal-resident discovery data but does
not enter the frontend Transcript or consume a semantic `JournalSequence`. If
its first append meets storage pressure, the existing gap policy keeps later
work volatile; the first successful recovery snapshot begins with the descriptor
and includes the complete semantic prefix.

Replay sequence orders every normalized storage record, while `JournalSequence`
orders only commands, events, and backend-correlation facts. The wire shape makes
that distinction structural: descriptors and message records cannot carry a
`journal_sequence`. Recovery indexes correlation records by their semantic
coordinate, validates every reference and binding transition, and publishes a
Continuation Anchor only when the accepted request and completed Turn are both
proven in the durable prefix.

The live producer now records the initial backend binding after `SessionCreated`,
uses each `SubmissionId` as the Start/Steer operation identity, and publishes
`TurnFinished(completed)`, its resumable outcome, and the Continuation Anchor in
one semantic commit. Provider adapters return opaque evidence without choosing
epochs or Journal coordinates; the runtime owns those semantic identities and
the Journal remains their sole sequence allocator. Transcript projection omits
the correlation-only records.

The Codex adapter preserves the user's effective model selection by omitting a
model override and records the `model` and `modelProvider` returned by
`thread/start`. It creates a persisted thread rather than an ephemeral one.
On continuation it decodes only its versioned Codex locator, sends exactly one
`thread/resume`, and verifies the returned thread, model-provider, and model
identities against the newest durable Anchor before the runtime publishes any
resumed state.

Pending message text is forced into an immutable segment before a non-text
ordering boundary, so concurrent Activity events can retain their original order.
If a crash leaves a message open, recovery proposes an interrupted seal after the
last durable record without discarding those interleaved events. A snapshot is
rejected before physical append when replay would need to synthesize a recovery
record, or after reopen when it omits the existing durable prefix and its
required recovery seals. A writer that directly observed its own failed append
may instead complete that retained prefix in one live-gap snapshot after all
open messages receive their actual terminal seals. Until then, later records
remain in the volatile suffix instead of turning expected snapshot deferral
into an integrity failure. Only capacity or storage-pressure failures enter
that automatic retry path. An
integrity gap or unexpected snapshot gate remains memory-only for the current
writer, so it cannot repeatedly propose an authority it cannot prove; a future
recovery owner must rebuild explicitly from the repository. These are
navigation notes for the implemented failure boundaries; the
owning behavior remains in the
[Session Journal](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.observability.session-journal.md)
and
[Session Repository](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.storage.session-repository.md)
KnowledgeUnits.

The CLI enables the local repository by default. `YO_SESSION_REPOSITORY`
overrides its root and `YO_SESSION_CAPACITY_BYTES` overrides the 1 GiB ceiling.
Linux otherwise uses `$XDG_STATE_HOME/yo/sessions` or
`$HOME/.local/state/yo/sessions`; macOS uses
`$HOME/Library/Application Support/yo/sessions`. The same root is opened without
creation or a writer lease by `yo session`. `yo --resume UUID` validates the
selected Session read-only first; an unavailable direct target opens its
archived Chat with a diagnostic instead of mutating storage. `yo --continue`
selects the newest eligible Session for the current Host and normalized
workspace and fails without creating a Session when none exists. A runnable
target is revalidated under its Session writer lease, restores the same Yo
Session identity, and resumes only its newest durable Anchor—there is no
fallback to an older Anchor. Remote storage, Request Audit persistence,
database or compression backends, and a durable transport remain outside this
composition.

## Suspend and resume

`Ctrl+Z` closes terminal ownership without closing the application Session:

```text
Ctrl+Z press
    ↓
TUI returns SuspendRequested after guarded terminal restoration
    ↓
TerminationCoordinator finalizes the active cleanup lease
    ├── termination selected: shut down the live agent and replay that signal
    └── no termination: return to Idle
          ↓
yo-cli applies default SIGTSTP and the process stops
          ↓ SIGCONT
restore inherited SIGTSTP state
          ↓
open a fresh active lease and terminal ownership generation
```

`TuiSession` and the same agent connection remain alive while the process is
stopped. Terminal input, raw mode, presenter, viewport ownership, and frame
history do not: each resumed generation reacquires them and starts with a full
frame. The retained appearance snapshot and revision also survive reentry; each
generation's first redraw pins that snapshot before measurement and carries it
unchanged through the completed `Surface`. `process/job_control.rs` temporarily
installs the default `SIGTSTP` action and restores the inherited action and mask
after continuation. The process host does not reconstruct or reselect the glyph
profile on resume.

The process may suspend only after `with_active_resource` has finalized the
cleanup lease with no selected termination signal. If a configured termination
arrives at that boundary, its resource-cleanup callback shuts down the retained
agent and that exact signal wins over suspension.

Contract: [Terminal job-control suspend and resume](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.job-control-suspend-resume.md)

## Exit and cleanup

User exit and process termination share the same cleanup route until the
process host applies signal policy:

```text
exit gesture or typed TerminationEvent
    ↓
yo-tui loop returns its reason
    ↓
terminal guard restores presentation state
    ↓
yo_tui::run_session_with_mode returns Exited
    ↓
AgentSession::shutdown
  stop worker → stop backend → close active semantic work
    ↓
TerminationCoordinator finishes the active resource lease
    ├── user exit: return to yo-cli
    └── signal: apply the selected signal's default disposition
          ↓
yo-cli restores installed signal state on ordinary return
```

For a completed application Session, the TUI reports `UserRequested` or
`TerminationRequested`; it neither identifies signals nor chooses their final
process behavior. Its guarded runner restores terminal state before returning
either outcome. `run_agent_generation` then calls agent shutdown even when the
terminal operation failed and aggregates both failures when necessary.

On an ordinary return, [`TerminationCoordinator::shutdown`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/termination/mod.rs)
restores installed signal dispositions and the installing thread's original
mask. On a selected termination signal, `with_active_resource` waits until the
TUI cleanup route has returned, invokes retained-agent cleanup when necessary,
then applies that signal's default disposition instead of turning it into a
normal application error.

## Finding the first owner

Follow the context nearest the first failed boundary:

| Visible context | Start here |
|---|---|
| `starting Codex` | `yo-core/backend/codex`, including transport startup |
| `creating the agent Session` | `yo-core/agent_session` startup and worker handshake |
| `terminal session` | `yo-tui/runner` and terminal mode cleanup |
| `agent cleanup` | `yo-core/agent_session::shutdown`, then runtime/backend cleanup |
| `process termination session` or `process termination cleanup` | `yo-cli/process/termination` |
| `suspending the process` | `yo-cli/process/job_control` |

Do not discard later cleanup failures: the current top-level path attempts the
independent cleanup boundaries and reports their contexts together.

## Contract owners

- [Command and event boundary](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.command-event-boundary.md)
- [Session, Turn, and Activity semantics](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.session-turn-activity.md)
- [Active-Turn input](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.active-turn-input.md)
- [Session Journal](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.observability.session-journal.md)
- [Session Repository](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.storage.session-repository.md)
- [Codex app-server adapter](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md)
- [Typed TUI flow](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.typed-flow.md)
- [Presentation mode selection](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.mode-selection.md)
- [Terminal lifecycle restoration](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.lifecycle-restoration.md)
- [Inline viewport publication](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.inline-viewport.md)
- [Process termination coordinator](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.process-termination-coordinator.md)
- [Terminal job-control suspend and resume](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.job-control-suspend-resume.md)

After locating the failing boundary, use [Validation](../validation/)
to choose the evidence that can confirm the fix.
