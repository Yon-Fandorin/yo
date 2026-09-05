# Code map

Use this map to choose an ownership boundary before searching for a concrete
type or function. It describes stable responsibilities and entry points, not
every source file.

## Cross-crate route

The process host creates the provider and frontend, then connects them through
frontend-independent session semantics:

```text
yo-cli main
├── yo-backend-delegated-codex CodexBackend
├── yo-backend-delegated-grok GrokBackend
├── yo-backend-managed NativeModelBackend
├── yo-cli TuiAgentConnection
└── yo-tui runner
        ↕ AgentConnection
    yo-core AgentSession
        ↕ bounded command lane + coalesced Journal-change lane
    worker-owned AgentRuntime
        ├── AgentEngine
        └── yo-core AgentBackend specialization
                ↕ yo-backend BackendAdapter + neutral evidence
```

The current implementation seams are:

- process policy and cleanup order live in `yo-cli`;
- generic backend lifecycle, evidence, and bounded child-process transport mechanisms live in
  `yo-backend`;
- the Connector-neutral managed model/tool loop lives in `yo-backend-managed`;
- Session, Turn, Activity, command, and event meaning live in `yo-core`; and
- terminal interaction and presentation live in `yo-tui`.

The accepted responsibilities and future-GUI constraint remain owned by the
[frontend-independent core boundary](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.core.frontend-independent-boundary.md),
[module and host boundaries](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.architecture.module-boundaries.md),
and [UI-only crate boundary](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.crate.ui-only-boundary.md).

## shared: cross-crate infrastructure

| Crate | Owns | Does not own |
|---|---|---|
| [`yo-yaml`](https://github.com/Yon-Fandorin/yo/blob/develop/shared/yo-yaml/src/lib.rs) | The workspace's safe-Rust YAML serialization boundary: exactly one document, finite event/node/depth/scalar/anchor/alias/replay budgets, bounded small aliases, duplicate/merge/unknown-alias/cycle rejection, and shared plain-scalar inference including YAML 1.1 booleans and `1_000` as an integer | Consumer schemas, model-profile inheritance, storage paths, Methexis or Librarian YAML migration, or format-version compatibility |

## yo-backend: backend foundation

| Boundary | Owns | Does not own |
|---|---|---|
| [`contract.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/foundation/src/contract.rs), [`evidence.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/foundation/src/evidence.rs), [`transport`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/foundation/src/transport.rs) | The generic `BackendAdapter` lifecycle, typed polling/cancellation/failure vocabulary, provider-neutral binding/request/outcome evidence, bounded semantic or opaque provider-private replay evidence, and bounded child-process JSONL, stderr-tail, request-ID, and deferred-message mechanics | Yo commands, events, Session or Journal coordinates, host protocol interpretation, Connector selection, or concrete Provider payload interpretation |

## yo-cli: process host

| Boundary | Owns | Does not own |
|---|---|---|
| [`src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs), [`src/lib.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/lib.rs), [`src/application.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/application.rs), [`src/application/`](https://github.com/Yon-Fandorin/yo/tree/develop/crates/yo-cli/src/application) | Public entry dispatch; live and one-shot lifecycle orchestration; the concrete TUI `AgentConnection` adapter; resume and continue selection; stdout/stderr finalization; and Codex warning publication | Command grammar, durable state encoding, backend semantics, or terminal rendering |
| [`src/command.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/command.rs), [`src/command/`](https://github.com/Yon-Fandorin/yo/tree/develop/crates/yo-cli/src/command) | Top-level grammar and vertical slices for account, connect, default, disconnect, model activation, live, print, Session inspection, and Usage output | Shared process lifecycle, storage implementation, or generic interaction primitives |
| [`src/execution.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/execution.rs), [`src/execution/`](https://github.com/Yon-Fandorin/yo/tree/develop/crates/yo-cli/src/execution) | Host-owned model selection and backend assembly; delegated-host verification; managed local tools; Unix job control and termination coordination | Command policy, durable connection mutation, or user-facing wording |
| [`src/interaction.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/interaction.rs), [`src/interaction/`](https://github.com/Yon-Fandorin/yo/tree/develop/crates/yo-cli/src/interaction) | Shared CLI styles, diagnostics, controlling-TTY prompts, and connection confirmation or result presentation | Command-specific output structure, repository mutation, or TUI rendering |
| [`src/state.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/state.rs), [`src/state/`](https://github.com/Yon-Fandorin/yo/tree/develop/crates/yo-cli/src/state) | General configuration capture, connection-operation and startup state, platform state-root selection, Session storage composition, and stable Host identity access | Physical Session semantics, Provider protocol behavior, or process execution policy |

Follow process-start or shutdown failures from `main.rs`, `lib.rs`, and `application/runtime/{startup,live,frontend,print}.rs` into the owner named
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
| [`host`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/host.rs) | An opaque random UUIDv4 `WorkspaceHostId`, its atomically created permission-restricted local per-user identity file, and the producing Host's lossless canonical workspace-path value | Remote Host transport and workspace comparison by matching Host identity |
| [`workspace_reference`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/workspace_reference.rs) | Frontend-neutral workspace-reference identity, provider port, revision-bound search messages, Unicode-normalized ranking, and the local execution provider's background Git-ignore-aware inventory | TUI presentation or submission-time admission; `yo-cli` only constructs the selected execution provider |
| [`skill_reference`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/skill_reference.rs) | Frontend-neutral skill identity, execution-environment provenance, catalog generation and entry revision selectors, availability, and revision-bound search messages | TUI presentation and exact submission-time revalidation; the concrete catalog adapter remains below `backend` |
| [`input`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/input.rs) | Immutable submitted text, ordered typed reference occurrences bound to exact visible byte spans, canonical safe reference-token projection, UUIDv4 submission correlation, and final whole-submission outcomes | `agent_session` for queueing and worker acceptance; concrete reference admission remains a later boundary |
| [`model_service`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service.rs), [`model_service/openrouter_discovery.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/openrouter_discovery.rs), [`model_service/qwencloud_catalog.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/qwencloud_catalog.rs) | Stable Provider, Account, and Model identities; explicit API-dialect selection with exactly one derived built-in connector; normalized HTTPS endpoints; Provider-and-Account base-profile plus whole-field model-override resolution into one complete typed profile, including a presence-aware optional known output maximum whose unknown form is omitted and whose whole-field null is invalid; bounded authenticated OpenRouter account-catalog transport and capability admission that retains every valid Model ID, narrows local-tools to no-tools when required, and exposes typed availability reasons; closed release-known QwenCloud and Kimi catalog seeds; one complete-binding value and closed durable decoder shared by startup and native resume; current pre-version `connections.yaml`, `credentials.yaml`, and connection-operation shapes decoded through `yo-yaml`; Provider-and-Account-scoped catalog, context-profile, and credential resolution; an injected tokenizer-counting port; redacted resolved credentials; the secure bounded local `credentials.yaml` exact-pair CAS with private revisions; the sole typed account, complete-model, catalog-seed, preference, and warning-only per-model failure-observation owner in bounded mode-`0600` `connections.yaml`; conditional observation CAS after exact binding and private credential-revision revalidation under the recovered operation lane; whole-pair replacement and exact-model mutations; and a secret-free bounded `connection-operation.yaml` intent repository with closed durable phases, structural binding admission without a connect-time model request, public-first disconnect execution, a pure exact-state recovery table, and a same-directory local executor that commits journal, credential, and public phases under one retained operation lock | General Provider discovery, Connector wire translation, command-level target and confirmation presentation, or the process host's configuration-path selection |
| [`model_service/connection_repository.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/connection_repository.rs), [`model_service/selection.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/selection.rs), [`model_service/startup.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/startup.rs) | Compatible absent-enabled/exact-false activation persistence outside complete-binding identity; exact-state activation CAS and preference clearing; disabled-row visibility; disjoint managed and revision-bound delegated-host picker targets; stable fingerprinted host AccountIds; account-section projection with the current section first and inline ` (current)`; same-account active-host availability plus disabled sibling projection; one typed `ModelBindingDisabled` admission failure for new startup, direct selection, picker acceptance, replacement, and reservation | CLI wording, host protocol decoding, credential mutation, or cross-runtime semantic handoff |
| [`model_service/kimi_catalog.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/kimi_catalog.rs), [`model_service/kimi_catalog`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/kimi_catalog) | Separate Platform AI and Code Membership catalog profiles and endpoints; one bounded authenticated Kimi `/models` snapshot; separate exact-once bounded Code Membership `/me` account-level and `/usages` account-capacity snapshots; first-valid duplicate handling; product-scoped reviewed exact-model overlays; complete selectable bindings; deterministic order; and visible typed reasons for every retained unavailable row | Kimi request/stream translation, private Session replay, Platform account capacity, or CLI consent and presentation |
| [`model_profile_admission.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_profile_admission.rs) | One shared admission of the resolved profile fields and local-tools/no-tools policy currently executable by the managed backend or external connection publication, plus the secret-free Kimi catalog/profile compatibility check required before connection mutation | Authored profile resolution, Provider wire serialization, connector transport, or durable binding identity |
| [`model_connector`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_connector.rs) | The public provider-neutral Connector and stream ports plus shared request, observation, cancellation, failure, limit, opaque provider-private envelope, and visible-projection vocabulary. Concrete Provider request, stream, and private payload grammars do not live in core | Provider wire grammar or a Yo-managed backend's semantic Activities and tool loop |
| [`connectors/transport`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/connectors/transport/src/lib.rs) | Reusable bounded HTTPS/SSE byte transport, cancellation, deadlines, redirects, backpressure, and worker cleanup without API-dialect interpretation | Provider request or event grammar |
| [`connectors/openai-responses`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/connectors/openai-responses/src/lib.rs) | The concrete OpenAI Responses implementation of the neutral Connector port, including request serialization and bounded response-event decoding. `yo-cli` now selects it for the exact Responses identity and injects it into the current managed loop | Model selection, credential resolution, shared transport bytes, or managed semantic Activities |
| [`connectors/openai-chat-completions`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/connectors/openai-chat-completions/src/lib.rs) | The concrete OpenAI-compatible Chat Completions implementation of the neutral Connector port, including ordered message and tool replay serialization plus its bounded finish–usage–`[DONE]` stream grammar. `yo-cli` selects it only for the exact Chat identity and dialect | Kimi private replay, model selection, shared transport bytes, or managed semantic Activities |
| [`connectors/kimi`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/connectors/kimi/src/lib.rs) | The concrete Kimi Chat Completions implementation of the neutral Connector port. It independently rechecks the Platform/Code matrix and exclusively owns Kimi request admission and serialization, stream grammar including exact cache-read usage availability, provider-private assistant codec and visible projection, and exact replay-size accounting; `yo-cli` selects it only for the exact Kimi identity and dialect | Catalog discovery and secret-free profile compatibility, durable opaque-envelope storage, shared transport bytes, or managed semantic Activities |
| [`tool`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/tool.rs) | Stable tool identities, frozen request registries, bounded argument validation against the closed `yo.tool-schema/v1` dialect, injected semantic admission for argument and output projections, normalized approval bindings, typed effects, and injected one-attempt execution-host ports | Concrete operating-system effects or provider-hosted tools |
| [`engine`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/engine.rs) | Deterministic Session, Turn, Activity, and request state transitions | `runtime` when a transition also crosses a provider boundary |
| [`journal`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/journal.rs), [`request_trace.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/request_trace.rs) | One ordered live projection of committed commands, semantic events, redacted context-policy/checkpoint observations, and internal backend-correlation facts; bounded sequence-based Transcript reads that omit correlation-only records; bounded payload-free Request-trace reads that retain correlation coordinates; the shared live/stored Request projection model; synchronous durable publication, typed gap state, bounded revision-aware `MessageSegment` construction, and recovery validation. Failed semantic outcomes persist an explicit nullable code beside their message. The private codec keeps semantic `JournalSequence` coordinates separate from physical replay coordinates, incrementally validates bounded replay chains, and validates backend exchanges, binding epochs, deterministic internal successor identities, accepted requests, resumable outcomes, Continuation Anchors, monotonic context policy, exact source groups, and atomic context checkpoints as one correlation graph | `runtime` for the capture point; `session_repository` for physical durability |
| [`session_repository`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/session_repository.rs) | Storage-neutral append, replay, and stored-Session discovery/read ports; snapshot recovery gate; typed storage pressure; and the first Session-single-writer local versioned-JSONL implementation. Multiple processes may open one stable root and write different Sessions concurrently. Each writer-capable instance retains a shared legacy compatibility guard; it acquires an exclusive lease before loading one Session and a short-lived root coordinator only for the final capacity check and physical append. Every current physical `v1` envelope carries a checksummed discovery summary. `LocalSessionReader` opens existing storage without a writer lease or mutation, lists from one validated tail envelope per Session, and captures one presence-aware point-in-time history read. `read_stored_session` keeps missing and present-but-incomplete histories distinct, validates physical envelopes and semantic recovery, coalesces storage-only message segments into semantic snapshots, and preserves message-recovery interruption, the first typed discovery mismatch with its physical sequence, and the fact that post-process durability continuity is not observable from `v1`. The same validated recovery derives a frontend-independent payload-free Request trace from every durable backend-correlation fact in Journal order; it exposes neither physical envelopes nor Request Audit payloads. Binding epochs and Continuation Anchors enter discovery only after semantic recovery proves their correlation chain; stored-history reads rederive the same state at each physical commit and reject a missing or contradictory summary. `read_stored_session_continuation` validates a candidate without mutation; `recover_stored_session_continuation` repeats that recovery under the Session writer lease and returns the descriptor, newest durable Anchor and binding evidence, restored semantic prefix, normalized frontend observations, next Turn identity, and admitted Submission identities as one typed unit. The local `reader` and `file` modules separate observation from mutation. `JournalRepository` validates a candidate against the durable semantic prefix and composes one semantic commit with one physical append | Provider-native resume, remote storage or transport, Request Audit persistence, and database or compression alternatives |
| [`runtime`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/runtime.rs) | Ordering backend acceptance, semantic commit, and Journal capture; owning binding and context epochs plus SubmissionId-derived command roots and distinct writer-assigned internal request identities; validating provider-neutral binding/request/outcome, monotonic completed active suffixes, and sequence-free checkpoint proposals; binding post-tool replay to a closed Journal Activity boundary; committing policy before first request and checkpoint before successor request acceptance; atomically publishing complete continuation chains and exact-replay or backend-native model-rebind transitions; reconstructing a deterministic Engine and exact replay-group state from a codec-validated durable prefix; verifying a resumed backend identity before publishing a full recovery snapshot; closing active work on failure | `backend/contract.rs` for the provider port |
| [`agent_session`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/agent_session.rs) | Nonblocking frontend access, bounded command lanes, stable submission identity across backpressure, an admission-time idle-compaction reservation that fences following prompts and binding replacement until checkpoint activation, worker-owned submission and nonterminal control outcomes, a capacity-one Journal-change notification including reservation release, startup cancellation, shutdown coordination, and startup hydration of the next Turn and admitted Submission identities from a validated continuation | `runtime` for worker-owned semantics |
| [`backend/contract.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/contract.rs), [`backend/evidence.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/evidence.rs) | Specializing `yo-backend::BackendAdapter` as `AgentBackend` with Yo's `AgentCommand`, `BackendEvent`, durable resume target, advertised native-model-rebind capability, and optional source anchor; retaining Session and Journal coordinates plus exact replay-profile/schema interpretation in core | Generic lifecycle/evidence mechanics, a concrete adapter, or Provider wire grammar |
| [`backends/delegated-codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-codex/src/lib.rs) | `codex app-server` launch and protocol classification, provider-ID correlation, translation into core events, account-bearing effective-model/thread continuation evidence, persisted rather than ephemeral threads, one verified `thread/resume` for the newest durable locator, one verified same-account exact-model `thread/fork` for native rebind, a Session-free authenticated `account/read` plus bounded paginated visible `model/list` inventory, and a worker-owned `skills/list` metadata catalog over the foundation transport | Provider-neutral runtime meaning, another delegated host, exact skill admission before structured dispatch, or atomic replacement policy |
| [`backends/delegated-grok`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-grok/src/lib.rs) | `grok agent stdio` launch, ACP v1 initialization, cached-login authentication, Session-free exact account and advertised `modelState` inventory snapshots, bounded request and Session correlation, permission mapping, event translation, `session/load` continuation, deterministic cleanup over the foundation transport, and no advertised state-preserving model-rebind capability | Provider-neutral runtime meaning, another delegated host, direct xAI Provider access, quota-window inference when the installed host reports none, or semantic handoff policy |
| [`backends/managed`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/managed/src/lib.rs), [`backends/managed/src/context.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/managed/src/context.rs) | The provider-neutral Yo-managed model loop over connector-neutral observations, semantic model/tool Activities, serial validation-admission-approval-execution ordering, cumulative retained-plus-current replay admission before every dispatch, bounded strictly decreasing request-cap selection and exact final-payload recounting, bounded model rounds, visible refusal replay, opaque provider-private replay admitted only after its completed visible projection and exact replay-profile schema match, one opaque Session-stable cache-affinity hint attached without a Provider branch, one durable `yo.model-usage-receipt/v1` per response and round with exact binding, token, and closed cache-read availability attribution, 85/90 exact context-pressure admission, one tools-disabled fixed-shape portable summary, completed post-tool active-suffix publication, post-summary recount, checkpoint-before-dispatch handshake, idle `/compact`, resume-safe context groups, cancellation cleanup, bounded replay deltas, typed pre-final context-exhaustion failure with binding latching, and the final-delta-only completed non-resumable exception without an anchor | Startup model selection, tokenizer implementations, provider-private payload interpretation, semantic-admission policy, or concrete local tool implementations |

`yo-backend::BackendAdapter` is the reusable transport-neutral port, while the
same foundation crate owns bounded child-process mechanics without host wire meaning.
`yo-core::AgentBackend` closes its associated types over Yo semantics and is the
current provider seam; Codex and Grok wire values live under their respective
`backends/delegated-*` crates, while the
Connector-neutral loop implements that seam from `yo-backend-managed`. The
[command and event boundary](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.runtime.command-event-boundary.md)
and [Codex app-server adapter](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md)
own the corresponding behavioral constraints. The
[model-service binding](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.model.service-binding.md)
and
[local account credential store](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.credentials.local-account-store.md)
own the provider-neutral identity and local-secret boundaries. The
[OpenAI Responses connector](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.connector.openai-responses.md)
and [OpenAI Chat Completions connector](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.connector.openai-chat-completions.md)
own their distinct remote grammars. `backends/managed` composes the dialect-derived connector with the frozen
tool registry and exact semantic replay; the process host selects and assembles
that backend from validated configuration. The connector itself does not own semantic Activities or
execute tools. The
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
Commands, events, and backend-correlation records carry their original explicit
`JournalSequence`; descriptors and normalized message records structurally omit
one. Recovery may accept gaps in semantic numbering, but never duplicates,
decreases, or an incremental value inside the preceding durable cutoff.
The descriptor consumes replay sequence 1 without inventing a semantic
`JournalSequence`; its own first physical envelope therefore has no semantic
cutoff. `JournalRepository`
validates new suffixes incrementally against its recovered state before mapping
them to the local repository instead of re-reading the JSONL log per append. It also derives the
descriptor carried by every physical discovery summary from that validated semantic prefix; the
current binding epoch and latest complete Continuation Anchor are derived from the same recovery
state rather than trusted from the envelope. The local writer adds `updated_unix_millis`
immediately before the same checksummed append. A live writer that
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
Chat, Transcript, or Request output. Transcript alone may select the newest
records and bound payload exposure. The separate `yo usage` command consumes
the same recovered history for a typed receipt report; neither command makes
any entry executable.

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
the latest value without choosing a visual presentation policy. Stored-Session
discovery, read-only history, and local delegated-host native resume are connected.
Resume still requires one complete durable Continuation Anchor, the same
Workspace Host, the recorded workspace, and exact returned backend,
model-provider, model, and thread identities; durability without those proofs
does not make a Session executable.
The local repository allows multiple live `yo` processes to open the same
default root and write different Sessions. One process owns each Session writer
lease, while a short root append coordinator keeps the shared capacity check
exact without remaining held between appends. A lifetime shared guard on the
legacy root lock fails closed when old and new writer binaries overlap.
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
| [`runner`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner.rs) | Public live-session facade, single terminal-owning loop, required readiness for every live source, indefinite idle waiting, configurable 120/60fps frame coalescing, a retained Inline Chat publication cursor, final cleanup reporting, and terminal-independent archived Chat, Transcript, and Request projection | `runner/state.rs` for semantic UI transitions and candidate orchestration; `runner/publication.rs` for persistent-row preparation and compact live size; `runner/frame.rs` for frame-rate policy; `runner/archival.rs` for stored output; `runner/unix.rs` and `runner/unix/presenter.rs` for live orchestration, post-flush geometry observation, and visible motion scheduling |
| [`runner/archival.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/archival.rs), [`runner/archival/request.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/archival/request.rs) | Read-only stored-Session output. Request renders the complete payload-free correlation trace in durable Journal order with its exact observation boundary, typed detail availability, and explicit unavailable Request Audit seam | Stored recovery or Request Audit persistence |
| [`appearance`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/appearance.rs) | Session-owned immutable appearance snapshots, monotonic revisions, resolved style roles, and the public built-in Rich/ASCII glyph profiles | `appearance/activity.rs` for validated activity-frame sequences, elapsed-time selection, maximum reserved marker width, continuous shimmer math, color-depth resolution, and reduced motion; `runner/session.rs` for profile-aware construction; `runner/state.rs` for frame pinning |
| [`plain`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/plain.rs) | Terminal-cell-aware plain lists that preserve pinned columns, pack short collapsed label/value pairs as a width-bounded flow, give block values an independent row and split their label from the value only when needed, wrap grapheme clusters without truncation, and fall back to a vertical card layout | Which columns mean what, their fold priorities or continuation hints, configuration, stdout TTY policy, or terminal ownership |
| [`meter`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/meter.rs) | Reusable terminal-safe percentage meters with vertical-level, horizontal-bar, and vertical-bar shapes, profile or custom glyph families, and validated layout templates | Calling presentation policy, semantic threshold colors, table column meaning, or terminal ownership |
| [`input`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/input.rs) | Decoded semantic key events, edit buffer, configurable bindings, exit gestures, prompt editing, typed view-switch presentation policy, and shared terminal notation for available key actions | `input/key_notation.rs` for terminal labels only; `prompt` for visible cursor layout; `runner/view.rs` for the selected projection |
| [`command.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/command.rs), [`definition.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/command/definition.rs), [`registry.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/command/registry.rs), [`palette.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/command/palette.rs) | One child module per built-in slash command owning its immutable definition; registry composition, uniqueness validation, lookup, and help projection; plus first-token query scanning, filtered prompt-overlay snapshots, and exact-draft Esc ownership in the palette lifecycle | `runner/state.rs` for input priority and effect execution; `overlay` for selection and presentation; `runner/model.rs` for `/model` resolution |
| [`runner/model.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/model.rs) | Projection of unified managed and delegated-host account sections into the shared selection panel; non-selectable section/status rows; exact model labels with inline current state; disabled-row visibility and typed managed-or-host acceptance | Durable activation mutation, host inventory transport, binding-transition policy, or generic overlay interaction |
| [`transcript`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/transcript.rs) | Ordered user and agent items, streaming revisions, separator-preserving range projection, transcript layout, and scrolling state | `runner/chat.rs` for the monotonic publication cursor; `shell` for compact composition with the prompt |
| [`runner/view.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/runner/view.rs) | Chat, Transcript, and Request selection; a header-free editable Chat surface; read-only mode headers; full Transcript projection; full-Session payload-free Request trace with optional exact-context highlighting; and mode-local context and viewport state | `runner/state.rs` for Journal observation and editor dispatch; `transcript` for shared layout and scrolling |
| [`prompt`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/prompt.rs) | Measuring and painting editor content plus cursor visibility; scanning eligible `@` and `$` tokens; preserving the last usable panel while a replacement query is pending; rejecting stale provider updates; replacing an accepted span; retaining its typed identity; and filtering cached skill candidates by reported scope | `input/editor` for edit semantics; the execution provider for discovery; `overlay` for freshness-gated presentation; `yo-core` for structured admission |
| [`overlay`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/overlay.rs) | Validated selectable-panel snapshots, semantic snapshot freshness independent of entry availability, typed static/activity title status, enabled-entry navigation and fitting, optional bottom-left filter presentation, atomic `Surface` paint, and a single prompt-overlay slot with token-and-revision presentation receipts that fence refreshed selection until the matching frame commits | Providers retain query, candidate filtering, preview, and accepted product effects; `runner/state.rs` owns routing and frame receipt commits; `shell` owns the bottom-anchored destination |
| [`shell`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/shell.rs) | Measuring the natural Inline live height with checked overflow reporting, allocating the work/prompt/metrics/help stack, fitting state-valid help as atomic priority segments, painting the pinned activity frame inside its maximum-width marker region plus the fixed-text style sheen, and reporting the shortest visible motion demand with the cursor from one completed frame | `shell/chrome.rs` for the work row, `shell/chrome/help.rs` for the footer; `input/key_notation.rs` for labels; `surface` for cell writes; `runner/session.rs` for honest host-known status values |
| [`surface`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/surface.rs), [`text`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/text.rs) | Adapter-independent cell state, Unicode graphemes and width, bounded views, diff spans, and terminal-independent text flow | `terminal` or `html` for projection |
| [`terminal`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal.rs) | Typed terminal operations and ANSI encoding | `terminal/mode` for presentation policy; `terminal/backend` for Unix effects |
| [`terminal/mode`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/mode.rs), [`terminal/backend`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/terminal/backend.rs) | Shared transactional restoration, Inline and Fullscreen presenters, Inline's typed-operation effect ledger with cursor-range and actual-scroll evidence, bounded write recovery, panic routing, and the crate-private direct unbuffered Unix output boundary | `terminal/mode/inline/transaction.rs` for operation/effect ordering and exact correction; `terminal/backend/unix` for exact downstream writes and post-flush events; `yo-cli/execution/process` only when process signal policy changes |
| [`html`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/html.rs) | Deterministic browser projection of completed `Surface` state | `surface` when terminal and browser output disagree |

`runner::TuiSession` owns the concise Chat transcript, editor, pending request,
three observability views, one token-scoped prompt-overlay slot and its pending
acceptance receipts, backpressured agent-dispatch state, one committed
appearance snapshot, and bounded recovered-publication evidence that can
outlive one terminal ownership generation. Chat is the editable default. F1,
F2, and F3 are the current typed presentation-policy
bindings for Chat, Transcript, and Request; the projection state does not own
those key choices. Usage is intentionally not a live F4 view; the completed
durable report is exposed by `yo usage SESSION_ID`. Transcript renders every committed command,
event, and redacted context policy/checkpoint observation received from the same
read-only Journal path; Chat turns a committed checkpoint into one concise
operator notice. Request renders every bounded correlation
record delivered by the live Request-trace reader in Journal order. The exact
Chat or Transcript context is only an optional highlight and never filters or
selects a nearby trace record; Request Audit remains explicitly unavailable.
Transcript and Request replace the prompt and consume input without dispatching
editor submissions. Each view retains its own context and viewport state.
The archived `yo session SESSION_ID --view request` path projects the same
bounded record model after validated stored-Session recovery and has no context
highlight.

Inline Chat moves only the maximal contiguous prefix of complete, stable items
into native terminal history. Preparation binds that candidate to the previous
publication cursor, appearance revision, terminal size, and geometry epoch;
only a completed downstream write acknowledges it. The remaining unpublished
suffix, prompt, chrome, and overlay form a compact live `Surface`. Detached Chat
review and the read-only Transcript and Request views freeze publication and use
the full terminal height; Fullscreen always renders complete semantic state.
After a successful flush, the Unix presenter drains queued resize notifications
and samples the terminal size. It may acknowledge the persistent prefix while
discarding stale live geometry, then immediately prepares a fresh suffix frame.
If the terminal transaction recovered an exact output error, the controller
retains its correction kind in `TuiSession::publication_recovery_evidence`.
Suspend emits no semantic suffix and preserves the cursor; normal exit and typed
termination append only still-unpublished semantic output.

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

The live `AgentConnection` now supplies ordered Transcript records, separate
durability transitions, and a payload-free Request trace that retains each
record's `JournalSequence`. Only the Transcript adapter drops those coordinates,
and Request Audit detail is unavailable, so the views expose those limits rather
than inferring missing values. This view layer does not persist Request Audit or
create another Journal owner; the worker-owned repository connection remains
below the frontend boundary.

Each redraw pins the appearance
revision before measurement and uses that same resolved snapshot through paint
and the completed `Surface`; plain session output pins the same session-owned
configuration. The runner supplies one generation-local elapsed sample. Appearance
selects the marker frame directly from that sample, keeps the first frame under
reduced motion, and retains the widest validated frame as a fixed marker region so
frame changes cannot move the label or alter fitting. A visible animated marker or
activity-text sheen returns its period, including a one-grapheme pulse; static,
hidden, empty, and reduced-motion indicators return no demand. The completed
frame reports the shortest positive period across its visible indicators.
`runner/unix.rs` derives the next epoch boundary and skips missed ticks;
`runner/frame.rs` folds a due motion request into the same readiness-driven
120/60fps frame boundary as input and background changes. Presenters and HTML
continue to consume only the completed `Surface`. Every public `TuiSession`
constructor and one-shot runner requires the process host to supply an explicit
TrueColor, Limited, or Unknown classification plus a Standard or Reduced motion
preference before appearance publication. `TuiSession::new` selects the default
Rich glyphs, while `TuiSession::with_glyph_profile` additionally lets the host
choose the built-in ASCII profile without exposing mutable theme state.
`TuiSession::with_session_info` adds backend and workspace labels to that same
explicit publication boundary. `TuiSession::with_frame_rate_limit` keeps the
default 120fps coalescing policy or lets the host lower it to 60fps without
changing semantic state transitions. The CLI maps startup-only
`tui.max_fps` configuration to that policy; a future GUI can reuse source
readiness while retaining its own event-loop and redraw/vsync policy;
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

Runtime scheduling contracts:
[bounded frame scheduling](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.frame-scheduling.md)
and
[fair readiness-driven event sources](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.runtime.event-source-scheduling.md).

Inline publication and compact live geometry are owned by the
[Inline viewport contract](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.terminal.inline-viewport.md).

The `surface` is the common completed state. Terminal and HTML projections
consume it independently; neither projection defines layout meaning for the
other.

## Repository development tools

[`tools/xtask`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/xtask/src/lib.rs)
owns structured checks that maintain this repository rather than the `yo`
product. Its checks classify changed paths and commit trailers for Slice review
and Developer Docs impact, and verify that Rust tests carry nearby explanatory
comments. The `slice::activation` module consumes a small semantic request,
pins current `develop`, publishes the canonical Methexis activation contract,
creates its Direct Slice worktree, and binds the two while recovering exact
partial setup. The review-packet modules use ordinary active ContextBuilds for
normal candidates and an explicitly versioned prospective operation for one
exact later activation request; the latter binds the proposed Checkpoint and
active-record transition without granting activation. Its bootstrap module
requires an exact versioned capability in trusted `develop` and admits only the
closed four-path activation transition, so implementation and workflow changes
remain on the ordinary review route. The `slice::close` module produces and applies a hash-addressed local
cleanup plan only after the accepted commit, Slice patch, review evidence,
binding, refs, and clean worktrees agree; its storage boundary rejects unsafe
plan-file inputs. `hk.pkl` decides when to run checks; `xtask` implements and
tests their rules. Methexis and Librarian retain their separate knowledge-domain
responsibilities, while simple external-command orchestration remains in `hk`
or a small validation script.

After choosing an owner, use [Validation](../validation/) as the
single map from changed boundary to evidence. Follow the
[terminal environment matrix](../validation/terminal-matrix.md) when real
terminal behavior is involved. Before closing a
[Slice](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#slice-contract),
widen checks only across the boundaries the change actually crosses.
