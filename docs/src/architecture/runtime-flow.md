# Runtime flow

Use these traces when a change crosses crate boundaries or when an error
message does not make its owner obvious. They describe the current
implementation path. Methexis remains the authority for what each boundary
must mean.

## Startup

The terminal is acquired only after process policy and the agent Session are
ready:

```text
yo-cli
  parse presentation mode and glyph profile; capture cwd
  install TerminationCoordinator
  open Host identity and Session repository
  normalize workspace and create SessionDescriptor
  spawn CodexBackend transport
      ↓
yo-core AgentSession
  start worker
  attempt descriptor envelope
  CreateSession
      ↓
Codex app-server
  initialize
  thread/start
      ↓
yo-core
  SessionCreated
      ↓
yo-tui
  acquire terminal and enter Inline or Fullscreen mode
```

| Step | Current owner | What to follow |
|---|---|---|
| 1 | [`yo-cli/src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs) | `run` selects presentation options, captures the working directory, installs termination coordination, opens Host identity plus Session storage, canonicalizes the workspace, and creates one matching UUIDv7 `SessionDescriptor`. |
| 2 | [`yo-core/backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs) | `CodexBackend::spawn` validates configuration and starts the stdio transport. It defers the provider handshake. |
| 3 | [`yo-core/agent_session`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/mod.rs) | `AgentSession::start_cancellable_with_repository` transfers the backend and local repository to the worker thread (named `yo-agent-runtime`) and waits for startup without blocking termination observation. |
| 4 | [`yo-core/agent_session/worker.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/worker.rs) | `AgentWorker::initialize` first attempts the descriptor-only Journal envelope, then sends `CreateSession` through `AgentRuntime`; storage pressure keeps both the descriptor and later activity in the recoverable volatile prefix. |
| 5 | [`yo-core/backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs) | `CreateSession` performs `initialize` and `thread/start`; the semantic engine produces `SessionCreated`. |
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
6. [`drain_agent` and `redraw`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/unix.rs)
   consume already committed Transcript records, update TUI state, compose a
   completed `Surface`, and send it to the active presenter. `runner/view.rs`
   selects Chat, Transcript, or Request from the same record stream. Chat shows
   user input only when its `StartTurn` or `SteerTurn` command appears in that
   sequence. Terminal `EventStream` readiness and the built-in agent, local
   workspace, and Codex skill producer readiness wake the owner thread. State changes request a frame
   instead of drawing synchronously for every event. `FrameScheduler` publishes
   the first and resize frames immediately, then coalesces ordinary requests at
   the `TuiSession` limit: 120fps by default, or 60fps when the host selects
   `FrameRateLimit::Fps60`. For those shipped asynchronous sources, the 50ms
   bounded wait is not an input, agent, provider, or rendering poll interval. It
   remains as a fallback for the process host's synchronous termination
   observation and for custom synchronous connections or providers that retain
   the default `poll_ready` implementation. An editor mutation that dispatches
   `@` or `$` discovery requests a frame before any provider result; the prior usable panel remains visible
   behind a pending snapshot gate. A Chat frame that actually paints the elapsed-selected
   Rich Braille or ASCII work-marker frame inside its fixed maximum-width region, or a
   fixed-text activity sheen, returns the shortest visible period;
   the runner schedules the next generation-epoch boundary and coalesces it with
   event redraw. Hidden, narrow, short, idle, reduced-motion, and zero-size indicators
   do not arm that timer. A one-grapheme activity status can still pulse and therefore
   remains animated.

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
explicit path. The first schema is:

```yaml
version: 1
session:
  list:
    date_format: "%Y-%m-%d %H:%M %:z"
```

The date syntax is strftime-compatible and both UPDATED and STARTED are shown
in the viewing machine's local timezone. Missing configuration uses the shown
default. Unreadable files, unsupported versions, unknown fields, oversized
files, and invalid date formats are explicit failures rather than silent
fallbacks. The reader opens one nonblocking descriptor, requires it to be a
regular file, and consumes at most 64 KiB plus one sentinel byte, so a FIFO
cannot stall the command and concurrent file growth cannot bypass the bound. A
missing repository produces an empty list and does not create state. Direct
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
single-writer versioned JSONL physical v1

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
An inherited pending marker blocks a successor writer, so only the writer that
created an in-flight marker can make its pre-append cutoff visible.

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
target is revalidated under the single-writer lease, restores the same Yo
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
- [Process termination coordinator](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.process-termination-coordinator.md)
- [Terminal job-control suspend and resume](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.job-control-suspend-resume.md)

After locating the failing boundary, use [Validation](../validation/)
to choose the evidence that can confirm the fix.
