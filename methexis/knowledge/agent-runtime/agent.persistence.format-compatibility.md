---
schema: methexis.knowledge/v1alpha1
id: agent.persistence.format-compatibility
kind: decision
owner: agent-runtime
sources:
  - id: agent.persistence-001
    revision: sha256:28d7fcb8c0d694c3d97ba384bad6dd54439df15e420dfb0b606b8add77b9c7d5
relations:
  depends_on:
    - agent.input.explicit-skill-reference
    - agent.input.workspace-reference
  applies_to:
    - yo-core::InputSubmission
    - yo-core::UserInput
    - yo-core::journal::SemanticRecord
    - yo-core::journal::codec
---
# Session persistence format compatibility

## Statement

The UUIDv7-only, descriptor-aware semantic Session Journal format
`yo.semantic-journal-commit/v1` and checksummed physical Session-record envelope
`yo.session-record/v1` are yo's initial public-format candidates. Before the
first public release, an earlier reviewed revision replaced the structured-input
semantic `/v1` with an anchored-session development shape. This second reviewed
revision replaces that immediately preceding shape with the closed
anchored-session semantic `/v1` defined below. Its exact shape and UUIDv7 Session identity are part of the
baseline; a matching schema tag alone MUST NOT admit a record.

Every semantic `/v1` commit, including a descriptor-only commit, MUST contain
the exact top-level discriminator `format: anchored-session`. A missing or
unknown value, or a Session history containing mixed format generations, MUST
fail closed before semantic admission.

Every persisted `command_committed`, `event_committed`,
`backend_exchange_observed`, `backend_binding_opened`,
`backend_binding_closed`, `backend_request_accepted`, `model_replay_delta`,
`backend_resumable_outcome`, and `continuation_anchor` record contains a
required positive `journal_sequence`. The sole Session Journal writer assigns
that identity when it commits the backend-neutral semantic record; a codec,
repository, retry, snapshot builder, or remote transport MUST NOT allocate or
renumber it. `session_descriptor`, `message_reset`, `message_segment`, and
`message_ended` are structurally separate persistence records and MUST NOT
contain `journal_sequence`. The field is required or forbidden by record type,
never nullable.

Persisted semantic JournalSequences MUST be unique and strictly increasing in
replay order across the Session, but they need not be contiguous. One or more
live text-update observations may be normalized into bounded message records,
so an intentional gap can represent semantic observations whose exact transport
chunk boundaries are not replay authority. `journal_cutoff` is the monotonic
semantic boundary durably covered after applying the commit. It is absent only
from the initial descriptor commit, MUST be positive otherwise, and MUST be no
less than every explicit `journal_sequence` represented by that durable state.
Every explicit sequence newly introduced by an incremental commit MUST be
strictly greater than the preceding durable `journal_cutoff`; a complete
snapshot is not an incremental commit and may restate only the exact sequence
values of the prefix it replaces.
Recovery MUST preserve explicit sequence values and MUST NOT fill gaps, infer
how many live deltas a message record represents, or renumber records to make a
contiguous projection.

A complete snapshot preserves the exact explicit JournalSequences and cutoff
of the state it replaces. Recovery rebuilds an in-memory
`JournalSequence -> semantic record` index from validated records; that index is
derived and MUST NOT be persisted as another authority. Duplicate or decreasing
explicit sequences, an incremental sequence at or below the preceding cutoff,
an explicit sequence beyond the current cutoff, a cutoff that moves backwards,
or any correlation or Anchor reference whose exact sequence is absent or has
the wrong record kind MUST fail closed. This makes JournalSequence the stable
semantic reference while storage-only ReplaySequence remains an internal
coordinate for normalized records within semantic payloads; RepositorySequence
separately orders physical Session-record appends.

`StartTurn` and `SteerTurn` command records MUST contain a `submission_id`
encoded as a canonical UUIDv4 string and an `input` object. A correlated
Activity user-input response uses the same `input` object without another
SubmissionId because its request identity already owns correlation. The closed
input object contains:

- `profile`: the exact value `yo.structured-input/v1`;
- `text`: the exact submitted UTF-8 string; and
- `references`: an ordered array of zero or more tagged occurrences.

The semantic replay domain MUST preserve a committed submission as its command
kind (`StartTurn` or `SteerTurn`), target Turn, SubmissionId, and `UserInput`.
The record itself is evidence that the exact submission was accepted. Recovery
and snapshots MUST preserve that correlation. Each SubmissionId MAY identify
at most one committed submission record in a Session; any second committed
occurrence is invalid, even when its other fields are byte-identical. SubmissionId is internal
correlation and MUST NOT be exposed in ordinary Chat or Transcript presentation
unless a later presentation contract explicitly selects it.

Every occurrence contains half-open `start` and `end` UTF-8 byte offsets into
`text` and the exact `projection` accepted during live capture. `start` and
`end` are JSON integers in the unsigned 64-bit domain; a decoder that cannot
address either value MUST reject the record. Occurrences MUST be non-empty, lie
on UTF-8 boundaries, and appear in strict non-overlapping order. `projection`
MUST be non-empty and byte-equal the referenced `text` span. The live writer
MUST validate that captured projection against the typed reference before
commit. Replay treats the typed identity, not projection text, as authority and
MUST NOT parse visible `@path` or `$name` text to recover identity. Future live
display-policy changes affect new capture only and MUST NOT reinterpret stored
projection bytes.

A `workspace` occurrence contains exactly `type: workspace`, `start`, `end`,
`projection`, `identity`,
`execution_environment_identity`, `workspace_identity`, `root_identity`,
`relative_path`, and `kind`. `kind` is exactly `file` or `directory`. A `skill`
occurrence contains exactly `type: skill`, `start`, `end`, `projection`, `identity`,
`execution_environment_identity`, `locator`, `name`, `scope`,
`catalog_generation`, and `entry_revision`. `scope` is exactly `workspace`,
`user`, `system`, or `admin`; `catalog_generation` is a positive unsigned 64-bit
JSON integer.

The immutable `yo.structured-input/v1` profile requires every identity,
execution-environment identity, workspace identity, root identity, locator,
name, and entry revision present for its occurrence to be non-empty. A workspace
`relative_path` MUST be non-empty, root-relative, `/`-separated, have no leading
or trailing `/`, and contain no empty, `.` or `..` component. It admits at most
one skill occurrence. Unknown input or occurrence fields, occurrence tags,
kinds, scopes, zero skill generations, invalid metadata, and invalid profile
values MUST fail closed. These rules define persisted `/v1`; later live-domain
rule changes MUST NOT silently change its decoder.

This second explicitly reviewed pre-release semantic `/v1` replaces the
immediately preceding anchored-session development shape. Logs written in the
displaced shape MUST fail closed even though the schema tag is unchanged. The
replacement admits one general exchange record and exactly six
continuation-specific records. Existing correlation records remain bounded and
payload-free. The separate `model_replay_delta` is bounded, payload-bearing
semantic Journal data rather than Request Audit detail:

- `backend_exchange_observed` contains positive `epoch`, canonical UUIDv4
  `operation_id`, exact `exchange_kind`, exact `direction`, `payload_schema`,
  optional positive `correlation_sequence`, optional `exchange_identity`, and
  exact `detail_availability` as defined below;

- `backend_binding_opened` contains positive `epoch`, `backend_kind`,
  `backend_version`, `binding_identity`, `model_identity`, `session_locator`,
  and `transition` objects defined below;
- `backend_binding_closed` contains the positive `epoch` being closed and exact
  `reason: replaced`, `revoked`, or `exhausted`;
- `backend_request_accepted` contains positive `epoch`, positive `turn_id`, the
  accepted submission's canonical UUIDv4 `operation_id`, and a
  positive `exchange_sequence` plus a `request_identity` object with `schema`
  and `value`;
- `model_replay_delta` contains positive `epoch`, positive `turn_id`, positive
  `accepted_request_sequence`, an optional replay contract, and an ordered
  non-empty list of exact provider-neutral replay items. The replay contract is
  present exactly once at the start of a replay chain and contains the exact
  system prompt plus ordered tools with name, safe description, schema version,
  and closed JSON schema. An item is exactly a message with role and visible
  UTF-8 content, a function call with call identity, tool name, and validated
  argument JSON bytes, or a function result with call identity and bounded
  model-visible output bytes;
- `backend_resumable_outcome` contains positive `epoch`, positive `turn_id`,
  positive `accepted_request_sequence`, positive `replay_delta_sequence`, exact
  `status: completed`, and an optional `outcome_identity` object with `schema`
  and `value`; and
- `continuation_anchor` contains positive `epoch`, positive
  `accepted_request_sequence`, positive `resumable_outcome_sequence`, and
  positive `journal_boundary`.

`exchange_kind` is exactly `request`, `response`, `notification`,
`server_request`, `retry`, or `terminal_outcome`. `direction` is exactly
`yo_to_backend` or `backend_to_yo`. `detail_availability` is exactly `persisted`,
`volatile`, `missing`, `unsupported`, `unpersisted`, or `redacted`; it describes
optional Request Audit detail and does not change the semantic exchange record's
authority. The exchange record's own JournalSequence is its observation
boundary.

The first exchange in an operation owns an `operation_id` that is unique in the
Yo Session. An outbound request originating from `StartTurn` or `SteerTurn`
MUST use that command's SubmissionId. Any other request, notification, or server
request receives a writer-assigned canonical UUIDv4; a backend-provided ID, when
present, belongs in `exchange_identity` instead. Root requests, notifications,
and server requests omit `correlation_sequence`. Every correlated exchange MUST
reuse the exact operation ID of the referenced exchange. A response MUST refer
to a request or server request in the opposite direction. A retry MUST refer to
a request, server request, or retry in the same direction. A terminal outcome
MUST refer to a request, server request, retry, or response in its operation
chain. Notifications never form correlation edges. All edges point backward
within the same epoch; no operation ID may begin a second root chain. These
closed edges keep requests, responses, notifications, server-initiated
requests, retries, and terminal outcomes distinguishable even when detail is
absent.

`binding_identity`, `model_identity`, `session_locator`, `exchange_identity`,
`request_identity`, and `outcome_identity` are versioned opaque objects with
exactly `schema` and `value`; only `exchange_identity` and `outcome_identity`
may be absent. `payload_schema`, `backend_kind`, and every object `schema`
are non-empty ASCII strings of at most 128 bytes. `backend_version` and every
backend-provided identity or locator `value` are non-empty UTF-8 strings of at
most 4096 bytes. The adapter selected by `backend_kind` owns interpretation of
those values. In particular, its binding-identity schema defines the exact
comparison used to verify the backend identity returned by native resume. The
shared semantic validator checks only the closed field shape, bounds, ordering,
and cross-record relationships; it MUST NOT parse a Codex, Kimi, remote-host,
or other backend-specific value.

The closed `transition` object contains exact `mode: initial`, `exact_replay`,
or `lossy_handoff`; exact `cache: not_applicable`, `lost`, or `unknown`; and an
optional positive `source_anchor_sequence`. `initial` requires
`cache: not_applicable` and no source Anchor. Both replacement modes require a
source Anchor in an earlier closed epoch. `exact_replay` requires `cache: lost`.
`lossy_handoff` requires `cache: lost` or `unknown` and marks the binding open
as the visible context-loss boundary. Its user-approved transformed-context
description remains ordinary semantic Journal data rather than an opaque
backend identity. The binding's backend and model identities, transition mode,
source Anchor, and cache state therefore remain available without Request Audit
detail.

The first binding epoch is 1 and MUST open after its backend Session has been
created and the matching semantic `SessionCreated` has been committed. Later
epochs increase by exactly one. At most one epoch is open, and a replacement
MUST close the current epoch with `reason: replaced` before opening its
successor. A process shutdown or restart alone does not close a resumable
binding: recovery retains the open epoch. Native resume continues that epoch
and MUST NOT emit another binding-open record. Before accepting another request,
the adapter MUST compare the resumed backend identity against the recorded
`binding_identity` under its versioned schema. A mismatch fails native resume;
the old epoch MUST close and an explicitly permitted replacement epoch MUST
open before another request can be accepted. A `backend_request_accepted` record
is valid only in the verified open epoch after yo has committed the matching
`StartTurn` or `SteerTurn` and observed its outbound request exchange. Its
`operation_id` MUST equal both that command's unique SubmissionId and the
referenced exchange's operation ID. `exchange_sequence` MUST identify the latest
`backend_exchange_observed(request, yo_to_backend)` for that operation and
epoch. Multiple accepted submissions MAY target one Turn; the request referenced
by a completed outcome MUST be the latest accepted request for that Turn in the
same epoch.

A `model_replay_delta` is valid only after a matching semantic
`TurnFinished` with outcome `completed`. It MUST reference the latest accepted
request for that Turn and epoch. Its message, function-call, and function-result
order and relationships MUST validate independently of presentation records or
old connector payloads. The encoded replay contract is limited to 1 MiB, one
delta to 16 MiB, and the replay prefix selected by an Anchor to 64 MiB and 4096
items. A bound violation produces a completed but non-resumable Turn and no
delta, outcome, or Anchor; later Turns on that binding fail with explicit
context exhaustion until an independently approved compaction or new binding.

A `backend_resumable_outcome` is valid only after that replay delta and a
matching semantic `TurnFinished` with outcome `completed`. It MUST reference the
latest accepted request for that Turn and epoch and the immediately preceding
replay delta. When a backend exposes a separate stable
result identity, `outcome_identity` records it. When it does not, omission is
explicit and the referenced accepted request identity remains the backend
operation identity; the writer MUST NOT invent a value. Failed or interrupted
Turns MUST NOT produce a resumable outcome.

A `continuation_anchor` MUST immediately follow its referenced resumable
outcome in the same semantic commit, and that outcome MUST immediately follow
its referenced replay delta. Every `*_sequence`,
`source_anchor_sequence`, `correlation_sequence`, and `journal_boundary` in
these seven records is a
semantic `JournalSequence`, never a storage-only ReplaySequence. The request
and outcome sequences MUST identify the correlated records in the same epoch,
and `journal_boundary` MUST equal the resumable outcome's JournalSequence. The
anchor record's own JournalSequence is the value projected into physical
discovery metadata. `TurnFinished(completed)`, the replay delta, the resumable outcome, and the
Anchor MUST occur in that order in one semantic commit. This
ordering makes the completed Turn, outcome, and Anchor one physical append
without making the Anchor circularly claim itself as its committed boundary.
Recovery and snapshots MUST preserve and revalidate the complete binding and
correlation graph. They MUST NOT synthesize an Anchor from a completed Turn,
discovery summary, backend wire payload, or Request Audit detail.

Every physical `/v1` record MUST contain a `discovery` object with:

- the complete Session descriptor: full UUIDv7 Session ID, workspace-host
  identity, host-normalized workspace path, and start time;
- writer-assigned `updated_unix_millis`;
- an optional binding epoch; and
- an optional latest valid Continuation Anchor `JournalSequence`.

The record's CRC32C MUST bind the complete discovery object in the same explicit
checksum preimage as its schema, Session identity, `RepositorySequence`, kind,
and exact payload bytes. It MUST NOT use a second checksum or a second append.

This reset explicitly supersedes every semantic `/v1` without the required
`format: anchored-session` discriminator, including the immediately preceding
structured-input and string-input semantic `/v1` shapes, the summary-less
physical `/v1` shape, and the development-only semantic meanings named
`yo.semantic-journal-commit/v1` through `/v4` and physical meanings named
`yo.session-record/v1` through `/v3`. Semantic `/v2`, `/v3`, and `/v4`, physical
`/v2` and `/v3`, and legacy numeric-identity records that reuse either `/v1`
tag MUST fail closed before semantic admission. They MUST NOT be migrated,
reinterpreted, skipped as valid history, or exposed as readable Session data.
Recovery MUST read only formats explicitly supported by an accepted
compatibility contract; at this baseline that set contains only the two current
closed `/v1` shapes. No legacy parser, dual reader, compatibility shim, or old
wire model is retained. A minimal rejection fixture MAY remain only to prove
that a displaced shape fails closed.

The current checksummed physical `yo.session-record/v1` envelope remains
unchanged because its CRC32C already binds the exact semantic payload bytes.
This contract governs Session Journal and Session-record persistence only.
Other persistent formats, including `yo.workspace-host-id/v1`, remain under
their own owning contracts.

Any further pre-release replacement under either `/v1` tag requires another
explicitly reviewed SOT revision that names the replaced shape and accepts its
data impact. After yo's first public release, evolution MUST preserve published
versions or provide an explicitly reviewed compatibility or migration contract;
it MUST NOT silently reset a published schema tag.


Every persisted failed semantic outcome MUST contain both a required `code`
field and a `message`. The code is either null or a non-empty ASCII identifier
of at most 128 bytes. Tool admission failures MUST use a stable non-null
`yo.tool.validation.*/v1` code; uncoded general failures are represented
explicitly as null rather than by omitting the field. The displaced message-only
failure shape MUST fail closed.

Replay arguments and outputs MUST pass the semantic-admission redaction gate
before becoming Activities, later model input, or durable replay. Prohibited raw
credentials, complete environment values, execution-host diagnostics, and
configured prohibited literals MUST NOT enter the semantic record. The admitted
exact value, including an explicit bounded replacement, is the sole replay value.

## Rationale

Reusing `v1` before the first release gives the public contract an honest
starting point without preserving experimental numbering. Naming the displaced
development schemas and making closed shape admission part of the baseline
prevents an old record with the same tag from being mistaken for current data.
Persisting SubmissionId and typed reference occurrences at capture time lets
replay recover the accepted input without guessing identity from display text.
Separate binding, accepted-request, and outcome records preserve backend
differences without duplicating their payloads in each Anchor. Backward
Journal-sequence references make a small Anchor verifiable, while the existing
envelope checksum protects the new semantic payload without creating a second
authority.
