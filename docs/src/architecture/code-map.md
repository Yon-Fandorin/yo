# Code map

Use this map to choose an ownership boundary before searching for a concrete
type or function. It describes stable responsibilities and entry points, not
every source file.

## Cross-crate route

The process host creates the provider and frontend, then connects them through
frontend-independent session semantics:

```text
yo-cli main
├── yo-core CodexBackend
├── yo-cli TuiAgentConnection
└── yo-tui runner
        ↕ AgentConnection
    yo-core AgentSession
        ↕ bounded command lane + coalesced Journal-change lane
    worker-owned AgentRuntime
        ├── AgentEngine
        └── AgentBackend
```

The current implementation seams are:

- process policy and cleanup order live in `yo-cli`;
- Session, Turn, Activity, command, and event meaning live in `yo-core`; and
- terminal interaction and presentation live in `yo-tui`.

The accepted responsibilities and future-GUI constraint remain owned by the
[frontend-independent core boundary](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.core.frontend-independent-boundary.md),
[module and host boundaries](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.architecture.module-boundaries.md),
and [UI-only crate boundary](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.crate.ui-only-boundary.md).

## yo-cli: process host

| Boundary | Owns | Does not own |
|---|---|---|
| [`src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs) | Argument parsing, presentation and glyph-profile selection before terminal acquisition, working-directory capture, provider startup, terminal-generation reentry, and top-level cleanup aggregation | Agent semantics or terminal rendering |
| [`src/agent/mod.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/agent/mod.rs) | Adapting `yo-core::AgentSession` to the TUI's `AgentConnection` port, including the concrete local Transcript cursor | Provider protocol translation or a premature local/remote reader trait |
| [`src/command.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/command.rs), [`src/session.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/session.rs), [`src/config.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/config.rs) | Separating live startup from `yo session` list/direct-read grammar; current-workspace or `--all` selection; Session-list date configuration; TTY-aware column priorities; and stdout/stderr routing for archived Chat/Transcript | Physical Session decode, semantic recovery, generic responsive plain-text layout, or executable continuation |
| [`src/storage.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/storage.rs) | Selecting the per-user platform state root, applying a separately overridable Session repository root, and composing separate local writer and non-creating reader paths. Writer startup establishes Host identity; read-only commands only observe an existing identity and repository | Host identity meaning or physical Session record semantics |
| [`src/process/job_control.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/job_control.rs) | Transactionally applying default `SIGTSTP`, suspending the process, and restoring inherited signal state after `SIGCONT` | TUI state or terminal restoration |
| [`src/process/termination`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/process/termination/mod.rs) | Unix signal installation, observation, restoration, and final disposition | Terminal state restoration |

Follow process-start or shutdown failures from `main.rs` into the owner named
in the error context. The signal coordinator lives in `yo-cli`; the TUI
receives only the typed, signal-neutral `TerminationEvent`. The
[process termination coordinator](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.process-termination-coordinator.md)
owns that contract.

## yo-core: agent semantics

[`src/lib.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/lib.rs)
is the public facade. Start there when a GUI, TUI, or provider adapter needs a
new shared capability.

| Module | Owns | Follow next |
|---|---|---|
| [`command.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/command.rs), [`event.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/event.rs), [`session.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/session.rs) | Provider-neutral commands, observable events, outcomes, typed identities, and the versioned `SessionDescriptor`; release-baseline Session identities are storage-independent UUIDv7 values whose embedded time matches the descriptor start time | `engine` for legal state transitions |
| [`host`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/host/mod.rs) | An opaque random UUIDv4 `WorkspaceHostId`, its atomically created permission-restricted local per-user identity file, and the producing Host's lossless canonical workspace-path value | Remote Host transport and workspace comparison by matching Host identity |
| [`workspace_reference`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/workspace_reference.rs) | Frontend-neutral workspace-reference identity, provider port, revision-bound search messages, Unicode-normalized ranking, and the local execution provider's background Git-ignore-aware inventory | TUI presentation or submission-time admission; `yo-cli` only constructs the selected execution provider |
| [`engine`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/engine/mod.rs) | Deterministic Session, Turn, Activity, and request state transitions | `runtime` when a transition also crosses a provider boundary |
| [`journal`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/journal/mod.rs) | One ordered live projection of committed commands and semantic events; bounded sequence-based Transcript reads; synchronous durable publication, typed gap state, bounded revision-aware `MessageSegment` construction, and recovery validation | `runtime` for the capture point; `session_repository` for physical durability |
| [`session_repository`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/session_repository/mod.rs) | Storage-neutral append, replay, and stored-Session discovery/read ports; snapshot recovery gate; typed storage pressure; and the first single-writer local versioned-JSONL implementation. Every current physical `v1` envelope carries a checksummed discovery summary. `LocalSessionReader` opens existing storage without a writer lease or mutation, lists from one validated tail envelope per Session, and captures one presence-aware point-in-time history read. `read_stored_session` keeps missing and present-but-incomplete histories distinct, validates physical envelopes and semantic recovery, coalesces storage-only message segments into semantic snapshots, and preserves message-recovery interruption, discovery disagreement, and the fact that post-process durability continuity is not observable from `v1` as typed history metadata. The local `reader` and `file` modules separate observation from mutation. `JournalRepository` validates a candidate against the durable semantic prefix and composes one semantic commit with one physical append | Executable continuation, remote storage or transport, Request Audit persistence, and database or compression alternatives |
| [`runtime`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/runtime/mod.rs) | Ordering backend acceptance, semantic commit, and Journal capture; translating backend observations; closing active work on failure | `backend/contract.rs` for the provider port |
| [`agent_session`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session/mod.rs) | Nonblocking frontend access, bounded command lanes, a capacity-one Journal-change notification, worker ownership, startup cancellation, and shutdown coordination | `runtime` for worker-owned semantics |
| [`backend/contract.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/contract.rs) | Provider capabilities, commands, semantic events, polling, cancellation, failure kinds, and explicit cleanup | A concrete adapter |
| [`backend/codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/mod.rs) | `codex app-server` lifecycle, JSON transport and protocol classification, provider-ID correlation, and translation into core events | `backend/contract.rs` before exposing new provider behavior |

`AgentBackend` is the current provider seam, and Codex wire values live under
`backend/codex`. The
[command and event boundary](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.command-event-boundary.md)
and [Codex app-server adapter](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md)
own the corresponding behavioral constraints. The
[Session Journal](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.observability.session-journal.md)
and
[Session Repository](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.storage.session-repository.md)
own the durable replay and storage contracts. The implemented composition
encodes semantic commits, bounds message content as `MessageSegment` records,
starts a new immutable message revision for an authoritative replacement
snapshot, forces pending text before a non-text ordering boundary, and
distinguishes same-writer live-gap snapshots from reopen recovery.
Before the first release, the codec writes and reads only semantic commit `v1`;
development-only predecessor formats are not compatibility promises.
`JournalSequence` remains the frontend-visible semantic cutoff while a private
replay coordinate orders the leading descriptor and normalized segment records.
The descriptor consumes replay sequence 1 without inventing a semantic
`JournalSequence`; its own first physical envelope therefore has no semantic
cutoff. `JournalRepository`
validates new suffixes incrementally against its recovered state before mapping
them to the local repository instead of re-reading the JSONL log per append. It also derives the
descriptor carried by every physical discovery summary from that validated semantic prefix; the
local writer adds `updated_unix_millis` immediately before the same checksummed append. A live writer that
observed its own storage-pressure failure may complete the retained prefix in
one snapshot; after reopen, a replacement snapshot must also retain required
recovery seals.

The live `AgentSession` worker now owns the `JournalRepository` call path. The
CLI establishes one durable local Workspace Host identity, opens the local
repository, canonicalizes the workspace without lossy UTF-8 conversion, and
creates a `SessionDescriptor` from one UUIDv7 clock reading.
`YO_SESSION_REPOSITORY` may relocate Session records without changing that Host
identity. The worker attempts the descriptor as the first Journal envelope
before backend `CreateSession`; if that append is unavailable, later activity
remains memory-only until one complete snapshot can publish the descriptor and
semantic prefix together. The storage-neutral `StoredSessionReader` now provides
bounded discovery, typed continuation eligibility, and durable history replay.
Unsupported schemas remain inspectable as unknown while quarantine and supported
records without an Anchor are unavailable. `yo session` consumes this read port
for current-workspace lists, `--all` discovery, and direct full-UUID read-only
Chat or Transcript output; it does not make any entry executable.

`yo-core` is still a `0.0.0` internal Pilot API. This Slice deliberately makes
persistent startup require a complete `SessionDescriptor` instead of a bare
`SessionId`, and lets `JournalDurability::Durable` carry no semantic cutoff while
only that descriptor is durable. `SessionId` is now UUIDv7-only, `as_uuid`
therefore returns a UUID directly, and `SessionDescriptor::for_session` is
infallible for an admitted identity. These are intentional source-breaking contract
corrections rather than compatibility shims; callers must migrate with this
repository before a public API is frozen.

The worker publishes durable records before their
committed semantic result is exposed. Streaming text remains a process-local live revision until a size,
time, ordering, or terminal boundary forces a durable segment or empty-revision
`MessageReset`. A known-clean capacity or storage-pressure refusal latches a
typed gap while the Session continues in memory; after open messages receive
real terminal seals, a later successful complete snapshot restores durability.
An ambiguous repository failure may instead become an integrity gap that this
writer does not retry automatically. The shared Transcript observation stream retains gap and
recovery transitions in order before their affected semantic records. The CLI
connection forwards those typed observations, and TUI state retains
the latest value without choosing a visual presentation policy. Stored-Session discovery and
read-only history are connected, but resume is not; durability alone does not
make the current CLI resumable.
The local repository's root-wide single-writer lock also means a second live
`yo` process cannot open the same default root. Separate multi-process writer
coordination is not part of this implementation.
Text admitted by a backend adapter as semantic `ModelWork`, including an
observable plan or reasoning summary, follows the same durable message path.
Hidden model reasoning yo never receives and unadmitted backend-specific
Request Audit payloads remain outside that semantic path.
Remote storage, Request Audit persistence, database or compression choices,
and a durable transport remain outside this path. `StoredSessionReader` is the
Session-specific read port, not a claimed common local/remote implementation;
extract transport-sharing machinery only when a real remote reader exists.

## yo-tui: terminal frontend

[`src/lib.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/lib.rs)
keeps the live runner facade narrow while exposing reusable completed-surface,
terminal-operation, and HTML-projection types.

| Module | Owns | Follow next |
|---|---|---|
| [`runner`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/mod.rs) | Public live-session facade, single terminal-owning loop, input/event orchestration, final cleanup reporting, and terminal-independent archived Chat/Transcript projection | `runner/state.rs` for semantic UI transitions; `runner/archival.rs` for stored output; `runner/unix.rs` for live orchestration |
| [`appearance`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/appearance/mod.rs) | Session-owned immutable appearance snapshots, monotonic revisions, resolved style roles, and the public built-in Rich/ASCII glyph and activity-motion profiles | `runner/session.rs` for profile-aware construction and ownership; `runner/state.rs` for frame pinning |
| [`plain`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/plain/mod.rs) | Terminal-cell-aware plain lists that preserve pinned columns, pack short collapsed label/value pairs as a width-bounded flow, give block values an independent row and split their label from the value only when needed, wrap grapheme clusters without truncation, and fall back to a vertical card layout | Which columns mean what, their fold priorities or continuation hints, configuration, stdout TTY policy, or terminal ownership |
| [`input`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/input/mod.rs) | Decoded semantic key events, edit buffer, configurable bindings, exit gestures, prompt editing, and the typed view-switch presentation policy | `prompt` for visible cursor layout; `runner/view.rs` for the selected projection |
| [`transcript`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/transcript/mod.rs) | Ordered user and agent items, streaming revisions, transcript layout, and scrolling state | `shell` for composition with the prompt |
| [`runner/view.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/view.rs) | Chat, Transcript, and Request selection; a header-free editable Chat surface; read-only mode headers; full Journal-record projection; exact Request anchoring and typed unavailable reasons; mode-local context and viewport state | `runner/state.rs` for Journal observation and editor dispatch; `transcript` for shared layout and scrolling |
| [`prompt`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/prompt/mod.rs) | Measuring and painting editor content plus cursor visibility; scanning eligible `@` tokens; rejecting stale provider updates; replacing an accepted span; and retaining its typed identity | `input/editor` for edit semantics; the execution provider for discovery; `yo-core` for structured admission |
| [`overlay`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/overlay/mod.rs) | Validated selectable-panel snapshots, enabled-entry navigation and fitting, atomic `Surface` paint, and a token-scoped single prompt-overlay slot | Providers retain query, filtering, preview, and accepted product effects; `runner/state.rs` owns routing and receipts; `shell` owns the bottom-anchored destination |
| [`shell`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/shell/mod.rs) | Allocating prompt-adjacent chrome, fitting typed status, painting the pinned activity marker, and reporting both the cursor and visible motion demand from one completed frame | `surface` for cell writes; `runner/unix.rs` for deadline scheduling; `runner/session.rs` for honest host-known status labels |
| [`surface`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/surface/mod.rs), [`text`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/text/mod.rs) | Adapter-independent cell state, Unicode graphemes and width, bounded views, diff spans, and terminal-independent text flow | `terminal` or `html` for projection |
| [`terminal`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/mod.rs) | Typed terminal operations and ANSI encoding | `terminal/mode` for presentation policy; `terminal/backend` for Unix effects |
| [`terminal/mode`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/mode/mod.rs), [`terminal/backend`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/backend/mod.rs) | Shared transactional restoration, Inline and Fullscreen presenters, panic routing, and the crate-private platform boundary | `yo-cli/process` only when process signal policy changes |
| [`html`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/html/mod.rs) | Deterministic browser projection of completed `Surface` state | `surface` when terminal and browser output disagree |

`runner::TuiSession` owns the concise Chat transcript, editor, pending request,
three observability views, one token-scoped prompt-overlay slot and its pending
acceptance receipts, backpressured agent-dispatch state, and one committed
appearance snapshot that can outlive one terminal ownership generation. Chat is
the editable default. F1, F2, and F3 are the current typed presentation-policy
bindings for Chat, Transcript, and Request; the projection state does not own
those key choices. Transcript renders every committed command and event received
from the same read-only Journal path. Request remains on the exact context
selected in Chat or Transcript and reports `no_associated_request` or
`request_audit_detail_unavailable` rather than searching adjacent records.
Transcript and Request replace the prompt and consume input without dispatching
editor submissions. Each view retains its own context and viewport state.

The accepted view semantics are owned by the
[Chat, Transcript, and Request projections](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.observability.view-projections.md)
contract; the prompt-adjacent regions and interruption affordances are owned by
the
[static input chrome](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.chrome.input-stack.md)
contract. Panel validation and paint are owned by the
[selection overlay](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.overlay.selection-panel.md)
contract, while token lifetime and input priority are owned by the
[prompt overlay routing](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.overlay.prompt-slot-routing.md)
contract.

The live `AgentConnection` now supplies ordered Transcript records and separate
durability transitions. The adapter still drops each record's `JournalSequence`,
and Request Audit detail is unavailable, so the views expose those limits rather
than inferring missing values. This view layer does not persist Request Audit or
create another Journal owner; the worker-owned repository connection remains
below the frontend boundary.

Each redraw pins the appearance
revision before measurement and uses that same resolved snapshot through paint
and the completed `Surface`; plain session output pins the same session-owned
configuration. The runner supplies one generation-local elapsed sample, and only
a frame that actually painted an animated marker returns a 120 ms motion demand.
`runner/unix.rs` derives the next epoch boundary, skips missed ticks, and folds
that deadline into normal and backpressured input waits; presenters and HTML
continue to consume only the completed `Surface`. `TuiSession::new` selects compatibility-default Rich glyphs,
while `TuiSession::with_glyph_profile` lets the process host choose the built-in
ASCII profile without exposing mutable theme state. `TuiSession::with_session_info`
also accepts the backend and workspace labels already known by the process host;
the chrome omits unavailable model, context, Git, and permission values instead
of inventing them. Reentry keeps the same
agent connection because the retained state contains identities from that agent
Session. `runner/unix.rs` acquires fresh terminal input, presenter, viewport
ownership, and frame history for each generation; those resources never move
into `TuiSession`. A clean `Ctrl+Z` returns
`TerminalOutcome::SuspendRequested` only after those generation-local resources
are restored.

Appearance contracts:
[session publication](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.session-publication.md),
[frame consistency](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.frame-consistency.md),
[glyph profiles](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.glyph-profiles.md),
[activity motion profile](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.appearance.activity-motion-profile.md),
[activity motion scheduling](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.activity-motion-scheduling.md),
and
[resolved cell style](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.resolved-style.md).

The `surface` is the common completed state. Terminal and HTML projections
consume it independently; neither projection defines layout meaning for the
other.

## Repository development tools

[`tools/xtask`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/xtask/src/lib.rs)
owns structured checks that maintain this repository rather than the `yo`
product. Its checks classify changed paths and commit trailers for Slice review
and Developer Docs impact, and verify that Rust tests carry nearby explanatory
comments. `hk.pkl` decides when to run them; `xtask` implements and tests their
rules. Methexis and Librarian retain their separate knowledge-domain
responsibilities, while simple external-command orchestration remains in `hk`
or a small validation script.

After choosing an owner, use [Validation](../validation/) as the
single map from changed boundary to evidence. Follow the
[terminal environment matrix](../validation/terminal-matrix.md) when real
terminal behavior is involved. Before closing a
[Slice](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#slice-contract),
widen checks only across the boundaries the change actually crosses.
