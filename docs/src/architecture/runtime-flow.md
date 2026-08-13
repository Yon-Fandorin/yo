# Runtime flow

Use these traces when a change crosses crate boundaries or when an error
message does not make its owner obvious. They describe the current
implementation path. Methexis remains the authority for what each boundary
must mean.

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
  ↓ injected tokenizer profile counts the exact serialized request
input admission with a reserved output budget

config.yaml sibling credentials.yaml
  ↓ one no-follow handle; regular file, current owner, 0600-equivalent,
    bounded size, stable metadata
immutable CredentialStore
  ↓ exact ProviderId + AccountId lookup
redacted ApiCredential
  ↓ dialect-derived OpenAiResponsesConnector or OpenAiChatCompletionsConnector
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
`yo-core::model_connector` derives one built-in connector from `api_dialect`.
Responses appends exactly one `responses` segment; Chat Completions appends
exactly `chat/completions`. Neither route adds another `v1`, probes a fallback,
or enables provider conversation authority or built-in tools. The Chat decoder
requires one index-zero choice, finish then final usage then `[DONE]`, preserves
content and refusal independently, and correlates indexed tool-call fragments.
Both dialects bound SSE events and cumulative payloads while reading, and
cancellation interrupts header, stream, and queue waits. `yo-core::backend::native` owns semantic Activities and the bounded
model/tool loop. Before each dispatch it counts the exact request with the
catalog-selected tokenizer profile. If the input budget or admitted replay
prefix is exhausted, it completes the current Turn without resumable evidence,
makes no over-budget remote request, and latches the binding against later
Turns. Raw tool
arguments are schema-validated and tool output is bounded before the injected
host gate decides the semantic form allowed into Activities, replay, and later
requests. The backend records only that admitted call/result replay, defers an
approved effect until its approval and attempt Activities can be journaled, and
attributes each terminal response's usage to its exact Provider, Account,
Model, connector, endpoint, and complete resolved profile. The process host owns startup
selection and assembly of these inputs and concrete local tools.

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
`connections.yaml` preference, which overrides operator `model.startup`.
Omitting one layer preserves the next present layer. When all three are absent,
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
  capture config.yaml and the non-creating stored preference for a new Session
  resolve invocation > stored preference > operator startup
  load the exact Provider/Account credential when a managed model is selected
  install TerminationCoordinator
  open Host identity and Session repository
  normalize workspace and create SessionDescriptor
  spawn CodexBackend transport or assemble the dialect-derived Yo-managed model backend
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
explicit path. The first schema is:

```yaml
version: 1
session:
  list:
    date_format: "%Y-%m-%d %H:%M %:z"
tui:
  max_fps: 120
model:
  startup:
    provider: qwencloud
    account: default
    model: qwen3.8-max
  bindings:
    - provider: qwencloud
      provider_display_name: QwenCloud
      account: default
      account_display_name: Default
      base_url: https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1
      profile:
        api_dialect: openai-responses
        tokenizer_profile: utf8-bytes/v1
        input_token_limit: 1000000
        max_output_tokens: 65536
        reasoning_parameters:
          effort: medium
        optional_request_parameters: {}
        tool_capability_policy: local-tools/v1
        verification_profile: semantic-terminal/v1
      models:
        - model: qwen3.8-max
          model_display_name: Qwen 3.8 Max
        - model: deepseek-v4-flash-0731
          model_display_name: DeepSeek V4 Flash
          profile:
            api_dialect: openai-chat-completions
            max_output_tokens: 8192
```

The date syntax is strftime-compatible and both UPDATED and STARTED are shown
in the viewing machine's local timezone. `tui.max_fps` accepts numeric `60` or
`120`; live startup reads it once and applies it to retained TUI generations.
Runtime reload is not supported. Operator `model.startup` accepts either exact
scalar `host:codex` or a `provider`, `account`, and `model` mapping that names
one catalog entry. Each `model.bindings` item owns one Provider-and-Account
endpoint and an optional base profile. A model inherits every base field and
replaces only fields present in its own `profile`; a structured field is one
whole replacement rather than a recursive merge. The resolved result must
contain all eight profile fields. Duplicate Provider-and-Account blocks,
unknown fields, duplicate structured keys, and incomplete results fail.
Structured numbers keep the variant selected by their authored spelling; an
explicit YAML numeric tag cannot retype that spelling. Startup and native
resume use the same closed durable complete-binding decoder, so out-of-range
JSON numbers cannot acquire a different variant at a later boundary.

The earlier flat `model.catalog` list remains readable under version 1 and
keeps its existing `yo.model-binding/v1` durable identity. It cannot appear in
the same document as `model.bindings`. A new explicit profile is attributed as
`yo.complete-model-binding/v1`; changing the endpoint, derived connector, or
any resolved profile field requires a new binding epoch on resume instead of
silently reusing the old one. Missing configuration retains built-in Session/TUI settings
but supplies no startup target, so live startup gives setup guidance instead of
silently selecting Codex. The YAML above is an operator-owned native model
example rather than an implicit model default.
Unreadable files, unsupported versions, unknown fields, oversized files,
invalid date formats, and unsupported frame rates are explicit failures rather than silent
fallbacks. The reader opens one no-follow nonblocking descriptor, requires it to be a
regular file, captures stable identity and metadata, and consumes at most 1 MiB plus one sentinel
byte, so a FIFO cannot stall the command and concurrent file growth cannot bypass the bound.
Preference mutations recapture this file and require the exact command-local snapshot to remain
unchanged before public commit. A
model API key is never read from an environment variable. When a configured
model is selected, Yo reads a separate `credentials.yaml` beside the selected
`config.yaml`. Its versioned Provider-then-Account shape is:

```yaml
version: 1
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
resolved. A revision-less version 1 file remains a readable legacy snapshot.
`LocalCredentialRepository` re-reads it under a private store lock and can
prepare exactly one add, replace, or remove without retaining the candidate
secret. Commit accepts the candidate only for add or replace, preserves every
unrelated pair, and atomically publishes a complete mode-`0600` snapshot with
an independently generated private `crev-...` receipt. The planned receipt and
exact pair state make a repeated commit idempotent, while a different observed
revision is a conflict. Removing the final pair leaves a versioned empty file
rather than returning to `absent`. These writes are a core storage boundary.

The sibling `connection-operation.yaml` owns the secret-free durable intent
for a credential-and-public operation. A closed version 1 record carries
an opaque operation ID, the config snapshot digest, a required legacy
`profile_digests` field that new writers leave empty, exact
expected and planned public revisions plus the complete bounded prospective
public snapshot, one add, replace, remove, or preserve credential receipt, and
one legal phase. Readers still validate and ignore non-empty values from an
older valid version 1 intent so an interrupted operation remains recoverable.
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
uses it to bind one selected managed target to the same Provider-and-Account
credential action and commits the public removal before any credential removal.

`yo connect Provider:Account:Model` accepts one exact configured reference. It
forms the union of every complete manual and currently managed binding for that
Provider and Account, plus the prospective selected binding. A retained legacy
binding fails before prompting because its full behavior cannot be verified.
The prospective managed upsert must also compose with the complete manual
catalog and pass startup-policy admission before any secret is read. Yo
requires confirmation, then reads one bounded API key only from the controlling
TTY with echo disabled and exact terminal settings
restored. If explicit restoration reports an error, the retained guard retries
restoration while unwinding. Environment,
arguments, standard input, and config files are not credential channels.
The confirmation presents the selected target, then uses stable semantic plan
markers (`+`, `~`, `−`, and `=`) to distinguish create, change, remove, and keep
effects. The default view keeps that decision-facing change set, the exact
target references for every profile verified with the key, and a concise plan
count. `-v` or `--verbose` additionally lists every distinct complete binding
in one structured exact-detail section with its non-secret endpoint, dialect,
and resolved profile fields. The credential row is derived
from the prepared repository action, so adding a new key and replacing an
existing key cannot share misleading copy. A checked success summary closes the
command. Color and emphasis augment those markers only on a terminal; `NO_COLOR`
and redirected standard output stay plain. The presenter reads the controlling
TTY width and wraps terminal-safe nonzero-width graphemes itself, preserving
exact non-secret value bytes rather than relying on the shell's incidental line
wrapping; an unavailable width uses an 80-column fallback.

The candidate key is used—without fallback to a stored key—to issue one bounded,
no-tool semantic request for every captured binding profile. Each verification
requires a completed message and completed terminal status. A completed visible
refusal is a valid semantic result; tool-call, incomplete, failed, closed-early,
or timeout outcomes fail verification. Diagnostics
retain only the non-secret target and connector failure class. After every
binding succeeds, the command revalidates the captured config, publishes a
secret-free intent, commits the exact add or replacement credential, advances
the journal, publishes the exact managed public snapshot, advances to complete,
and clears the journal. A crash after credential commit resumes only the stored
public bytes and never reconstructs or re-verifies a secret.

`yo disconnect` interactively infers a unique managed target or asks for one
exact captured `Provider:Account:Model` reference. Automatic execution requires
`yo disconnect PROVIDER --account ACCOUNT --yes` and proceeds only when that
pair has exactly one managed target; `--yes` never guesses among multiple
models. A manual-only match directs the operator to edit `config.yaml` because
the command removes only managed provenance. Before confirmation, Yo composes
the prospective managed removal with the captured manual catalog. The compact
default preview uses the same semantic plan markers for the managed removal,
default and API-key changes, and new- versus saved-Session effects. Its API-key
row names every remaining model that still depends on that key. `-v` or
`--verbose` also shows the exact removed complete binding, provenance
transition, and remaining bindings for the pair. The preview resolves the prospective
startup layers and names the exact lower-priority target for new Sessions, or
states that no target remains; it does not infer that behavior from preference
removal alone. Remaining account models are compact references, so an equal
manual binding is visible without repeating the removed profile. The same
controlling-TTY width boundary keeps every preview row within the observed width.
An equal manual binding remains manual and therefore preserves the
credential. Only an empty post-removal dependent set prepares credential
removal; an absent credential fails before intent rather than inventing state.
After confirmation and the final config guard, the command publishes the
secret-free intent, commits the public removal, advances `public_committed`,
optionally removes the credential, advances to `complete`, and clears the
journal. Existing Session history is not deleted, but a Session attributed to
the removed complete binding may no longer resume natively unless an equal
manual binding remains or the exact binding is reconnected; the preview states
that continuation result separately from stored-history preservation.

Endpoint, model, API dialect, derived connector identity, the resolved profile, and display
names remain non-secret binding data rather than secret-file content. Catalog limits and model
IDs are operator-owned examples and must be checked against the exact current
Provider offering. `utf8-bytes/v1` conservatively counts the complete serialized
request one token per UTF-8 byte; `o200k_base/v1` is available only for bindings
whose tokenizer is actually o200k-compatible. Unknown profiles fail startup.
`max_output_tokens` is both the wire output cap and the amount excluded from the
configured input limit during local context admission. The first explicit
runtime supports an empty reasoning mapping or an `effort` of `none`,
`minimal`, `medium`, or `high`; it requires empty
`optional_request_parameters`, `local-tools/v1`, and
`semantic-terminal/v1`. Other validated profile identifiers remain readable
configuration but fail startup until their runtime behavior exists.

The public sibling `connections.yaml` is separate from operator-owned
`config.yaml` and secret `credentials.yaml`. It stores one typed managed account
list, one flat complete-binding list, and the selection-owned preference. A
representative snapshot is below (the opaque revision value is illustrative):

```yaml
version: 1
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
      verification_profile: semantic-terminal/v1
accounts:
  - provider: qwencloud
    provider_display_name: QwenCloud
    account: default
    account_display_name: Default
```

An absent file is the canonical unset snapshot and is read without creating a
directory. Capture rejects unknown fields, duplicate account or binding
coordinates, bindings without their account, inconsistent Provider display
metadata, invalid complete bindings, and out-of-range unquoted structured-profile
numbers. The same scalar-style validator protects manual and managed YAML, so a
quoted numeric-looking string remains a string while a plain invalid number
cannot be retyped silently.

Managed upsert adds or replaces one exact complete coordinate, preserves every
unrelated entry, and publishes the first ModelTarget preference only from an
unset capture. Managed removal removes one exact binding, drops its account only
when no managed sibling still uses that pair, and clears only an exact matching
ModelTarget preference. Preference-only preparation preserves both managed
arrays byte-semantically. All mutations reserve one new opaque revision and use
the existing old-or-exact-new CAS. An absent first write uses same-directory
exclusive publication; later writes use durable atomic replacement. Exact
planned revision and bytes are idempotent success, while another revision is a
conflict.
Credential-changing managed connect reserves a new public revision even when
the visible binding bytes are otherwise equal, giving recovery an exact public
epoch for a key rotation without changing unrelated state or an existing
preference.

Every live startup captures `config.yaml` and `connections.yaml`, then composes
manual and managed entries by complete-binding equality. Equal entries coalesce
while retaining `manual-and-managed` provenance; manual display metadata wins
and managed display fills omissions. A field difference at the same Provider,
Account, and Model returns `BindingConflict` with the non-secret differing field
names instead of selecting a source. The composed catalog supplies initial
selection, resume matching, and the live model picker.

`yo default TARGET`, `yo default --unset`, explicit `yo connect host:codex`,
external model connect, and external model disconnect use one nonblocking
process operation lock and resolve pending multi-repository work before reading
new command configuration. The
preference-only commands publish one public CAS after target admission or Local
Codex verification plus the final configuration guard; they do not create a new
operation journal or inspect credential revisions, and re-encoding preserves
managed entries. External connect and disconnect use their operation-specific
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
