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
`NO_COLOR`.

Contracts:
[session publication](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.session-publication.md),
[frame consistency](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.frame-consistency.md),
[glyph profiles](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.glyph-profiles.md),
and
[resolved cell style](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.resolved-style.md).

## One active turn

A submitted prompt follows this route:

```text
terminal input
    ↓
TuiState::handle
    ↓ AgentIntent::Submit
TuiAgentConnection
    ↓
AgentSession admission and bounded command lane
    ↓
AgentWorker
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
TuiAgentConnection + TranscriptReader
    ↓ ordered AgentPoll::Record
    ↓
TuiState::observe_record
    ├── concise Chat projection
    └── chronological Transcript / anchored Request projections
          ↓ selected view
completed Surface
    ↓
Inline or Fullscreen presenter
```

The useful inspection points are:

1. [`TuiState::handle`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/state.rs)
   clears the submitted prompt and emits the frontend-neutral
   `AgentIntent::Submit`. It does not display that input as committed history.
2. [`TuiAgentConnection`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/agent/mod.rs)
   is a narrow local adapter. It forwards dispatch and retry operations, turns
   a coalesced Session change notification into bounded `TranscriptReader`
   suffix reads, and exposes ordered records to the TUI. It owns no Session or
   provider semantics.
3. [`agent_session/admission.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/admission.rs)
   resolves Submit to `StartTurn` or `SteerTurn`. A busy state lock or full
   bounded lane returns an opaque pending command for the TUI loop to retry.
4. [`AgentWorker`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/worker.rs)
   is the only owner that executes and polls the runtime. The terminal-owning
   thread does not wait on provider I/O.
5. [`AgentRuntime`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/runtime/mod.rs)
   orders command validation, backend acceptance, semantic commit, and Journal
   publication. The worker-owned durable writer maps text updates to bounded
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
   coalesced worker wake-up cannot erase a Gap-to-Durable transition. The CLI
   adapter forwards that order to TUI state with the exact cutoff class. Chat, status-row, or banner presentation
   remains a separate product contract. Stored-Session
   discovery and resume are also not current runtime behavior.
6. [`drain_agent` and `redraw`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/unix.rs)
   consume already committed Transcript records, update TUI state, compose a
   completed `Surface`, and send it to the active presenter. `runner/view.rs`
   selects Chat, Transcript, or Request from the same record stream. Chat shows
   user input only when its `StartTurn` or `SteerTurn` command appears in that
   sequence.

The change lane carries no command or event payload and has capacity one.
Multiple commits may therefore share one unread wake-up without losing
history: the concrete local reader continues by Journal sequence until it
reaches the observed head. A terminal backend failure is reported only after
the adapter has exposed the failure records already committed to the Journal.

Codex JSON and provider identifiers end at the backend adapter. Terminal input
events and rendering types end in `yo-tui`. The command and event types crossing
the middle are owned by `yo-core`.

## Live observation views

The selected TUI projection changes presentation, not Session authority:

```text
read-only AgentPoll::Record stream
    ├── Chat: concise activity/message projection + editable prompt
    └── full semantic record projection
          ├── Transcript: chronological command/event and Activity detail
          └── Request: exact Chat/Transcript context anchor
                ├── direct ActivityRequestRef → Request Audit unavailable
                └── no direct correlation → no associated request
```

F1/F2/F3 currently select Chat/Transcript/Request through
`input/view_binding.rs`. That mapping is a typed presentation-policy seam, not
projection state. Page and line navigation update the active view's own
viewport; Chat and Transcript also retain their own context cursor. Request
navigation scrolls only the anchored diagnostic page, so it cannot become a
nearby-request browser. Returning to a view restores its retained state when
its anchor is unchanged.

All three modes use the session's pinned appearance snapshot and the existing
Transcript layout and Surface primitives. The status row shows the active mode
and keys, switches to a compact `[C]123`, `[T]123`, or `[R]123` form on narrow
frames, and remains renderable when only one terminal row is available.
Transcript and Request are full-page read-only modes: their input path never
reaches the prompt editor or emits a submission.

The current TUI adapter exposes semantic `TranscriptRecord` values and typed
durability transitions, but still drops the reader's per-record `JournalSequence`
and does not expose Request Audit detail. Transcript prints that observation boundary. Request
uses only a correlation carried by its exact record and otherwise reports
`no_associated_request`; when an exact `ActivityRequestRef` exists it reports
`request_audit_detail_unavailable`. It never borrows a correlation from an
adjacent record. The repository is now inside the live worker path, while these
additional observation coordinates remain to be wired into the frontend
contract.

## Durable Journal composition seam

The live `AgentSession` uses this local composition:

```text
initial SessionDescriptor (replay sequence 1, no semantic cutoff)
    ↓
semantic Journal records
    ↓ bounded MessageSegment construction
JournalCommit codec
    ↓ one semantic commit
JournalRepository
    ↓ validate with durable semantic prefix
    ↓ one physical append
SessionRepository
    ↓
single-writer versioned JSONL

versioned JSONL
    ↓ bounded suffix read + semantic decode
Journal recovery
    ↓
RecoveredJournal or an explicit recovery error
```

Before the backend receives `CreateSession`, the worker attempts one
descriptor-only incremental envelope containing the UUIDv7 Session identity,
Workspace Host identity, the producing Host's canonical path bytes, and the
matching start time. The descriptor is Journal-resident discovery data but does
not enter the frontend Transcript or consume a semantic `JournalSequence`. If
its first append meets storage pressure, the existing gap policy keeps later
work volatile; the first successful recovery snapshot begins with the descriptor
and includes the complete semantic prefix.

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
`$HOME/Library/Application Support/yo/sessions`. This composition does not yet
add stored-Session opening, remote storage, Request Audit persistence, database
or compression backends, or a durable transport.

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
