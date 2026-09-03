---
schema: methexis.knowledge/v1alpha1
id: agent.persistence.format-compatibility
kind: decision
owner: agent-runtime
sources:
  - id: agent.persistence-001
    revision: sha256:99e90bd799d103a41cac35932d10a610d552316c547c76d5ec0cf787ce01a75b
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
semantic `/v1` with an anchored-session development shape. A second reviewed revision then replaced that shape with the replay-delta
development shape. A third reviewed revision replaced that immediately
preceding shape with a continuation-strategy-aware anchored-session shape. This
fourth reviewed revision explicitly extends that same shape with the optional
assistant-refusal replay field defined below. This fifth reviewed revision
additively extends the same pre-release semantic shape with one provider-private
replay item and replay-profile evidence while retaining the physical v1 envelope.
This sixth reviewed revision replaces the unimplemented
`context_compaction_handoff` development proposal with same-binding
`context_checkpoint` and context-epoch evidence. The displaced handoff shape is
unreadable development data rather than an accepted alias. This seventh reviewed
revision additively extends the same anchored-session shape with the
`backend_native_model_rebind` transition and its failure-atomic lineage evidence.
The physical v1 envelope remains unchanged. Its exact shape and UUIDv7 Session
identity are part of the
baseline; a matching schema tag alone MUST NOT admit a record.

Every semantic `/v1` commit, including a descriptor-only commit, MUST contain
the exact top-level discriminator `format: anchored-session`. A missing or
unknown value, or a Session history containing different top-level `format`
discriminator values, MUST
fail closed before semantic admission.

Every persisted `command_committed`, `event_committed`,
`backend_exchange_observed`, `backend_binding_opened`,
`backend_binding_closed`, `backend_request_accepted`, `model_replay_delta`,
`backend_resumable_outcome`, `continuation_anchor`, `context_policy_changed`,
and `context_checkpoint` record contains a
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

This third explicitly reviewed pre-release semantic `/v1` replaces the
immediately preceding replay-delta development shape. Logs written in the
displaced shape MUST fail closed even though the schema tag is unchanged. The
replacement admits one general exchange record and exactly eight
continuation-and-context records. Existing correlation records remain bounded and
payload-free. The separate `model_replay_delta` is bounded, payload-bearing
semantic Journal data rather than Request Audit detail:

- `backend_exchange_observed` contains positive `epoch`, canonical UUIDv4
  `operation_id`, exact `exchange_kind`, exact `direction`, `payload_schema`,
  optional positive `correlation_sequence`, optional `exchange_identity`, and
  exact `detail_availability` as defined below;

- `backend_binding_opened` contains positive `epoch`, `backend_kind`,
  `backend_version`, `binding_identity`, `model_identity`, `session_locator`,
  and `transition` and `continuation_strategy` objects defined below;
- `backend_binding_closed` contains the positive `epoch` being closed and exact
  `reason: replaced`, `revoked`, or `exhausted`;
- `backend_request_accepted` contains positive `epoch`, positive
  `context_epoch`, positive `turn_id`, the
  accepted submission's canonical UUIDv4 `operation_id`, and a
  positive `exchange_sequence` plus a `request_identity` object with `schema`
  and `value`;
- `model_replay_delta` contains positive `epoch`, positive `context_epoch`,
  positive `turn_id`, positive
  `accepted_request_sequence`, an optional replay contract, and an ordered
  non-empty list of exact replay items. At binding open, the replay contract is
  present exactly once at the start of that binding's initial replay chain. A
  same-binding checkpoint carries the exact replay contract for its successor
  chain, so later deltas in that exact binding and successor context epoch omit
  it until another checkpoint or binding transition. The replay contract contains the exact
  system prompt plus ordered tools with name, safe description, schema version,
  and closed JSON schema. An item is exactly a message with role, visible UTF-8
  content, and an optional independent visible refusal; a function call with
  call identity, tool name, and validated argument JSON bytes; or a function
  result with call identity and bounded model-visible output bytes; or the
  provider-private assistant item defined below. Refusal is
  valid only on an assistant message. Absence means no refusal. When present,
  refusal MUST be a non-null JSON string containing UTF-8, including the valid
  empty string `""`; null and every non-string value MUST fail closed. Content
  and refusal preserve their exact decoded UTF-8 bytes independently;
- `backend_resumable_outcome` contains positive `epoch`, positive
  `context_epoch`, positive `turn_id`,
  positive `accepted_request_sequence`, optional positive
  `replay_delta_sequence`, exact `status: completed`, and an optional
  `outcome_identity` object with `schema` and `value`; and
- `continuation_anchor` contains positive `epoch`, positive `context_epoch`, positive
  `accepted_request_sequence`, positive `resumable_outcome_sequence`, and
  positive `journal_boundary`;
- `context_policy_changed` contains `profile` with exact value
  `yo.context-policy/v1alpha1`, positive `policy_revision`, boolean `enabled`,
  exact `strategy`, integer `warning_percent`, integer `trigger_percent`, and
  the optional retained-raw fields defined below; and
- `context_checkpoint` contains `profile` with exact value
  `yo.context-checkpoint/v1alpha1`, the positive binding `epoch`, positive
  `previous_context_epoch`, positive `successor_context_epoch`, positive
  `source_anchor_sequence`, positive `source_journal_boundary`, positive
  `policy_revision`, exact `strategy`, positive `input_token_limit`,
  non-negative `input_tokens_before` and `input_tokens_after`, required
  `replay_contract` with the closed replay-contract shape defined above, and the closed
  summary, retained-group, receipt, loss, and summary-usage fields defined
  below.

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

The closed `continuation_strategy` object is exactly either an exact-replay
object with required `mode: exact_replay`, required `executor: local_client |
managed_server`, and the optional `replay_profile` defined below, or exact
`{ mode: backend_managed_state }`. Unknown fields and values fail closed, and
`backend_managed_state` forbids `executor` and `replay_profile`. The strategy is explicit
binding evidence; validators MUST NOT infer it from `backend_kind`, Provider,
API dialect, model, or locator. It is distinct from the `transition.mode`
below: transition records how a new epoch was seeded, while continuation strategy
records who owns context reconstruction for later requests in that epoch.

The extended exact-replay form MAY additionally contain one non-null
`replay_profile` string. Absence is the exact preceding representation and
normalizes to `semantic-only/v1`; a current producer omits the field for that
value. Presence is initially valid only as exact
`kimi-private-local-plaintext/v1`, which declares semantic item schema
`kimi.assistant-message/v1alpha1`; a current producer MUST emit it for that
profile. `backend_managed_state` forbids `replay_profile`. Unknown, null,
empty, or other values fail closed. This normalized value is part of the
versioned `binding_identity` comparison and epoch evidence. Before committing
the binding-open record, the selected adapter MUST prove that it equals the
complete effective binding's resolved replay profile. The shared semantic
validator checks the closed field and cross-record uses but never derives it
from a ModelId, Connector, or opaque binding value.

The closed provider-private replay variant has exact `kind:
provider_private_assistant`, exact non-null `schema:
kimi.assistant-message/v1alpha1`, positive `binding_epoch`, and `message`; no
other item field is admitted. `binding_epoch` MUST equal the containing replay
delta or checkpoint binding epoch, whose open binding MUST carry exact replay profile
`kimi-private-local-plaintext/v1`. It is a backend binding epoch, never a
context epoch. A same-binding checkpoint
therefore preserves an inline retained private item with that unchanged
`binding_epoch` across the context-epoch increment; only a binding-epoch
mismatch is cross-binding-epoch private state. The closed message object contains exactly
required `role: assistant`, required UTF-8 string `reasoning_content`, required
`content` as either a UTF-8 string or null, and optional `tool_calls`. Absent or
null reasoning, an absent content field, and every unknown field fail closed.
When present, `tool_calls` is a non-empty ordered array of at most 1,024 items.
Each element contains exactly `id` as 1 to 4,096 UTF-8 bytes, exact `type:
function`, and `function`; the function object contains exactly `name` as 3 to
64 ASCII bytes matching `^[a-zA-Z_][a-zA-Z0-9-_]{2,63}$` and `arguments` as at
most 4,194,304 UTF-8 bytes that parse as one JSON value. IDs are unique in the
assistant group and must equal their generic function-call counterparts. Null,
duplicate, malformed, or unknown fields fail closed.

One private item MUST immediately follow the matching generic assistant message
and its zero or more contiguous function-call items, before any function result
or later message. Its content string projects byte-for-byte to the generic
assistant content; null projects to an empty generic content and is valid only
when no visible content fragment existed. Its tool calls project in order and
field-for-field to those generic function-call items; absence is valid only
when there are none. The generic assistant refusal MUST be absent. A mismatch,
second private item for the same assistant group, unpaired private item, or
private item under another replay profile fails the complete delta. The private
message replaces that generic assistant group only during the exact Kimi
private-replay Connector projection, so one complete assistant object rather
than two is sent; the
generic items remain the frontend-neutral visible replay authority.

Both exact-replay executors use the same replay item, contract, bounds, digest,
ordering, and Anchor validation. `local_client` assembles the next request from
the local repository. `managed_server` reserves that assembly for a future
Yo-managed Session service and MUST NOT be emitted by the current implementation.
Its future admission additionally requires verified server and repository
identity, replay boundary, replay-content and contract digests, binding epoch,
availability, and retention under an independently reviewed contract.

The closed `transition` object contains exact `mode: initial`, `exact_replay`,
`lossy_handoff`, or `backend_native_model_rebind`; exact cache value
`not_applicable`, `lost`, or `unknown`; and optional positive
`source_anchor_sequence` and `source_checkpoint_sequence`. `initial` requires
`cache: not_applicable` and neither source coordinate. `exact_replay` and
`lossy_handoff` require exactly one source coordinate in an earlier closed epoch.
A source Anchor after a checkpoint names the reconstruction whose lineage starts
at that checkpoint. A source checkpoint is valid only when it is the newest
executable reconstruction root and no later request was accepted in its binding.
`exact_replay` requires `cache: lost`. `lossy_handoff` requires `cache: lost`
or `unknown` and marks the binding open as the visible context-loss boundary.
Its user-approved transformed-context description remains ordinary semantic
Journal data rather than an opaque backend identity.

`backend_native_model_rebind` requires exact `cache: unknown`, forbids
`source_checkpoint_sequence`, and is valid only between two
`backend_managed_state` bindings for the same delegated Host and verified
authenticated account. When the source epoch contains any accepted backend
request, it requires `source_anchor_sequence` naming that epoch's newest durable
Continuation Anchor. It may omit both source coordinates only when the live source
epoch contains no accepted backend request. An accepted request without a newest
matching Anchor makes the transition invalid rather than permitting omission.
The target locator MUST differ from the source locator, the target model identity
MUST equal the exact model confirmed after mutation, and the adapter MUST verify
same-Host and same-account binding evidence before publication. The shared codec
retains those identities as opaque values and validates the closed transition
shape and epoch graph; it does not parse HostId, HostAccountId, or HostModelId.

The native-rebind candidate is prepared through one live advertised
state-preserving fork plus model-mutation capability while the source epoch stays
open. Only the atomic Journal commit closes the source with `reason: replaced`
and opens the successor epoch. Unsupported capability, stale inventory, account
drift, a reused locator, missing confirmation, a mismatched model, or publication
failure MUST leave the source epoch and locator unchanged and executable. The
unbound candidate is discarded or quarantined and MUST NOT become continuation
authority. This transition claims neither semantic replay nor cache restoration,
and it MUST NOT admit cross-Host, cross-account, in-place source mutation, or an
unadvertised private mutation method. The binding's backend and model identities,
transition mode, optional source Anchor, and cache state therefore remain
available without Request Audit detail.

When the selected replay reconstruction named by a source Anchor or source
checkpoint contains a provider-private item, a replacement
`transition.mode: exact_replay` is valid only when the
target records the same complete binding identity and replay profile or an
independently reviewed lossless-conversion schema. Without such a converter,
every different target MUST use `lossy_handoff`; dropping the item while
recording exact replay fails semantic admission even when the target itself uses
`semantic-only/v1`.

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
open before another request can be accepted.

Context epoch is a separate positive Session-global counter whose initial value
is 1. Every binding opened after the initial binding inherits the current
context epoch unchanged. Only a valid same-binding `context_checkpoint`
increments it by exactly one. Every `backend_request_accepted` MUST carry the
context epoch current at its dispatch. An active Turn may span context epochs
only across a checkpoint committed between complete correlated semantic groups
and before its next ordinary Turn request. Its terminal `model_replay_delta`,
`backend_resumable_outcome`, and `continuation_anchor` MUST use the newest
context epoch and latest accepted request in that epoch; their binding and
context epoch pair MUST match. Earlier accepted requests in the same Turn
remain historical evidence under their original epochs. A request, delta,
outcome, or Anchor whose JournalSequence is later than a checkpoint MUST name
that checkpoint's successor context epoch; naming its previous epoch is stale
and MUST fail closed even though its binding epoch still matches. Historical
records at or before the checkpoint's source boundary remain valid,
byte-identical evidence and MUST NOT be rejected merely because their context
epoch has since been superseded.

Context policy revision is a separate positive Session-global counter. The
sole writer MUST commit revision 1 before the first
`backend_request_accepted`; every replacement increments it by exactly one and
becomes current at its own JournalSequence. `strategy` is exactly
`portable-summary/v1alpha1` or `exact-replay-only/v1alpha1`.
`warning_percent` is an integer from 1 through 99, `trigger_percent` is an
integer from 2 through 100, and warning MUST be strictly lower than trigger.
`retained_raw_percent`, when present, is an integer from 1 through 100;
`retained_raw_max_tokens`, when present, is positive. The retained-raw fields
are permitted only with `portable-summary/v1alpha1` and bound only additional
older groups; `exact-replay-only/v1alpha1` MUST omit both. Boolean `enabled`
does not erase the selected strategy or its valid bounds: false disables both
automatic and manual compaction, while preserving a closed policy that can be
re-enabled only by another policy revision. Unknown fields, strategies,
invalid combinations, skipped or repeated revisions, or an accepted request
before revision 1 fail closed. A checkpoint MUST name the exact policy revision
and strategy current at its JournalSequence.

A `backend_request_accepted` record
is valid only in the verified open epoch after yo has committed the matching
`StartTurn` or `SteerTurn` and observed its outbound request exchange. Its
`operation_id` MUST equal both that command's unique SubmissionId and the
referenced exchange's operation ID. `exchange_sequence` MUST identify the latest
`backend_exchange_observed(request, yo_to_backend)` for that operation and
epoch. Multiple accepted submissions MAY target one Turn; the request referenced
by a completed outcome MUST be the latest accepted request for that Turn in the
same epoch.

A `model_replay_delta` is valid only for an `exact_replay` binding and only
after a matching semantic `TurnFinished` with outcome `completed`. A
`backend_managed_state` binding MUST NOT emit one. It MUST reference the latest accepted
request for that Turn and epoch. Without a checkpoint in the Turn, it contains
the Turn's complete model-visible replay addition. If the Turn crosses one or
more checkpoints, it contains exactly the non-empty model-visible suffix
committed after the newest checkpoint record and MUST NOT duplicate that
checkpoint's replay contract, synthetic body, retained groups, or any earlier
Turn item. Its message, function-call, and function-result order and
relationships MUST validate both within the suffix and against the checkpoint
root independently of presentation records or old connector payloads. The
encoded replay contract is limited to 1 MiB, one
delta to 16 MiB, and the replay prefix selected by an Anchor to 64 MiB and 4096
items. Replay-prefix or model-context capacity exhaustion discovered before a
final assistant answer is accepted produces a typed failed non-resumable Turn
and no delta, outcome, or Anchor. Only when one complete final assistant answer
and every required semantic and provider-private item have passed their
individual validation and bounds may cumulative replay-application exhaustion
against the retained prefix preserve a completed but non-resumable Turn without
those continuation records. Later Turns on that binding fail with explicit
context exhaustion until an independently approved compaction or new binding.
Message content and refusal are each limited to 16 MiB of decoded UTF-8 octets.
The existing delta and replay-prefix limits measure the complete canonical
encoded delta bytes after JSON escaping. A refusal on a system, developer, or
user message MUST fail closed during evidence validation and wire decoding
rather than being reinterpreted by a connector.

The complete canonical JSON encoding of one provider-private item is limited to
16 MiB. It counts once inside the containing delta's 16-MiB ceiling or the
checkpoint retained groups' 64-MiB prefix ceiling, never in addition to the
applicable limit. Reasoning, content, IDs, names, and argument fragments
are checked incrementally before an excess byte is retained; their final JSON
escaping is included in the canonical delta metric. Snapshots preserve the
exact item, its relative order, replay profile, and epoch and revalidate the
same projection and bounds. A failed private admission rejects the whole replay
container and therefore prevents its delta or checkpoint commit and any later
outcome and Anchor rather than dropping or redacting only the private item.

A `backend_resumable_outcome` is valid only after a matching semantic
`TurnFinished` with outcome `completed` and MUST reference the latest accepted
request for that Turn and epoch. For an `exact_replay` binding,
`replay_delta_sequence` is required and MUST reference the immediately preceding
replay delta. For a `backend_managed_state` binding it is forbidden; the outcome
remains payload-free and relies on the binding, accepted-request, outcome, and
backend-session identities. When a backend exposes a separate stable
result identity, `outcome_identity` records it. When it does not, omission is
explicit and the referenced accepted request identity remains the backend
operation identity; the writer MUST NOT invent a value. Failed or interrupted
Turns MUST NOT produce a resumable outcome.

A `continuation_anchor` MUST immediately follow its referenced resumable
outcome in the same semantic commit. For an `exact_replay` binding, that outcome
MUST immediately follow its referenced replay delta. For a
`backend_managed_state` binding, the outcome MUST immediately follow the matching
`TurnFinished(completed)` and no replay delta may intervene. Every `*_sequence`,
`source_anchor_sequence`, `source_checkpoint_sequence`, `correlation_sequence`, and `journal_boundary` in
these nine record kinds is a
semantic `JournalSequence`, never a storage-only ReplaySequence. The request
and outcome sequences MUST identify the correlated records in the same epoch,
and `journal_boundary` MUST equal the resumable outcome's JournalSequence. The
anchor record's own JournalSequence is the value projected into physical
discovery metadata. For `exact_replay`, `TurnFinished(completed)`, replay delta, resumable outcome,
and Anchor MUST occur in that order in one semantic commit. For
`backend_managed_state`, `TurnFinished(completed)`, resumable outcome, and Anchor
MUST occur in that order in one semantic commit. This
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
compatibility contract; at this baseline that set contains only the current
closed semantic and physical `/v1` shapes, including the additive replay-item
extension defined here. No legacy parser, dual reader, compatibility shim, or
old wire model is retained. A minimal rejection fixture MAY remain only to
prove that a displaced shape fails closed.

The current checksummed physical `yo.session-record/v1` envelope remains
unchanged because its CRC32C already binds the exact semantic payload bytes.
This contract governs Session Journal and Session-record persistence only.
Other persistent formats, including `yo.workspace-host-id/v1`, remain under
their own owning contracts.

This fourth explicitly reviewed pre-release semantic `/v1` change is an
additive extension of the continuation-strategy-aware anchored-session shape,
not another distinguishable format generation. It adds only the optional
`refusal` field to a replay message and retains `format: anchored-session`; it
does not change the physical envelope or any other semantic record. Every valid
refusal-absent record from the preceding revision is also a current-generation
record under this extended closed shape, so mixing refusal-absent and
refusal-bearing messages does not violate the top-level format-generation rule.
The new reader MUST accept every valid preceding record by interpreting an
absent field as no refusal. A reader for the preceding closed shape rejects a
new refusal-bearing record as an unknown field. Consequently, an existing
Session remains readable by the preceding reader only until a refusal-bearing
replay delta is persisted; after that point a downgrade fails closed for that
Session. No migration, dual write, or downgrade compatibility shim is provided.
This asymmetric pre-release data impact is explicitly accepted by this
revision.

This fifth explicitly reviewed pre-release semantic `/v1` change is another
additive extension of that same `format: anchored-session` generation. The
physical `yo.session-record/v1` schema, exact top-level fields, record-kind
grammar, discovery object, `crc32c/v1` representation, and checksum domain and
preimage remain byte-for-byte unchanged; the already-bound payload string may
now contain the closed private item and replay-profile evidence above. Every
valid preceding semantic record remains valid. The current reader accepts a
Session log containing preceding deltas and later private-bearing deltas without
rewriting either. A preceding semantic reader rejects the new item variant or
replay-profile field as unknown, so a Session remains downgrade-readable only
until either is persisted. This accepted asymmetric pre-release impact provides
no migration, dual write, item omission, or downgrade shim. Exact fixtures MUST
prove unchanged preceding bytes, current mixed-history recovery and snapshot,
preceding-reader failure on both new shapes, canonical encoded bound accounting,
CRC coverage of the extended payload, and rejection of every null, omission,
unknown-field, order, projection, schema, profile, epoch, and duplicate case
defined above.

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

A bounded `context_checkpoint` record has no fields beyond the closed list in
the record grammar above, including its required replay contract, plus
`portable_body`, `retained_groups`, optional
`first_retained_sequence`, `artifact_receipts`, `losses`, and `summary_usage`.
`portable_body` is the validated Markdown UTF-8 string under the existing
16-MiB message-content bound. `retained_groups` is an ordered array of closed
objects containing only positive inclusive `first_sequence` and
`last_sequence` plus an ordered non-empty `items` array using the exact replay-item
grammar above. Source ranges MUST be non-overlapping, strictly increasing,
identify whole correlated semantic groups at or before
`source_journal_boundary`, and belong to the named binding and previous context
epoch. They are provenance, not replay-byte locators: the inline items committed
by the sole semantic writer are the durable replay authority and recovery MUST
NOT derive them from Activities, presentation, or ReplaySequence. Every group
MUST validate its internal call/result and provider-private relationships. Across
all groups there are at most 4,096 items and their complete canonical encoding,
together with the synthetic body, MUST fit the 64-MiB replay-prefix bound. The optional
`first_retained_sequence` is present exactly when the array is non-empty and
equals its first range's `first_sequence`.

Each `artifact_receipts` item contains only `profile` with exact value
`yo.context-artifact-receipt/v1alpha1`, lowercase `sha256:<64-hex>`
`content_hash`, positive `byte_count`, non-empty ASCII `media_kind` of at most
128 bytes, positive `source_context_epoch`, and positive
`source_journal_sequence`. Its source MUST be an eligible visible output in the
summarized prefix of the same Session, named previous context epoch, and source
boundary. The receipt is operator disclosure only in this revision and MUST NOT
appear in `portable_body`, retained-group items, or reconstructed Connector
input; raw bytes, a replay placeholder, expiry, a path, another Session, or
retrieval authority are not fields. `losses` is an ordered bounded list of
exactly either `visible_prefix_summarized` with positive inclusive source
sequence bounds, or `provider_private_dropped` with non-empty bounded ASCII
`schema`, exact `present: true`, positive `byte_count`, and positive
`source_journal_sequence`. Unknown variants or fields fail closed.
`summary_usage` is exactly one closed `yo.model-usage-receipt/v1` content object
for the summary response under the backend usage contract; it retains its
source attribution and reported availability objects without embedding a raw
wire response.

The successor context epoch MUST equal the previous value plus one without
overflow. `source_anchor_sequence` MUST identify a valid Anchor in the same
binding and previous context epoch, and `source_journal_boundary` MUST be at
least that Anchor's committed boundary and no later than the checkpoint's
preceding semantic sequence. Every retained, receipt, and loss coordinate MUST
be inside that exact boundary. `policy_revision` and `strategy` MUST equal the
policy current immediately before the checkpoint. Provider-private state,
private reasoning, credentials, uncommitted effects, arbitrary filesystem
paths, and raw artifact bytes MUST NOT enter the checkpoint. A retained group
that originally contains a required provider-private assistant item remains
indivisible and reconstructs that exact Journal-backed item; a private item in
the summarized prefix is intentionally absent from successor replay and is
represented only by `provider_private_dropped`.

The sole semantic writer MUST append that one checkpoint atomically without a `backend_binding_closed` or `backend_binding_opened` record. Its required replay contract MUST equal the exact canonical system and ordered tool contract used by the binding at the source boundary. During an active Turn, the source boundary MUST follow a completely committed correlated semantic group and precede both the checkpoint and the next accepted ordinary Turn request; it MUST NOT split a model response, pending tool effect, or incomplete group. Before commit, the writer MUST encode every selected retained group from its already admitted semantic replay state directly into that group's inline `items`; after commit, the checkpoint is their sole durable replay authority even when the Turn has not finished. Only after durable commit may model-visible replay become exactly one synthetic `user` message whose content is `portable_body`, followed by those exact inline retained-group items in original order under that replay contract. The checkpoint changes neither complete binding identity nor the binding transition's cache field and MUST NOT claim that Provider cache was preserved or lost; a later terminal ModelWork usage receipt is the sole evidence for reported cache reads. Failure, cancellation before commit, malformed summary, artifact-integrity failure, another `Compact`, typed `Reject`, or durability failure MUST append no checkpoint and leaves every original record and epoch authoritative and byte-unchanged.

Recovery and snapshots MUST validate the complete source-Anchor, boundary, replay-contract, inline retained-group, receipt, loss, policy, usage, and paired epoch graph. They MUST apply a checkpoint only when its previous context epoch is current in its named open binding and replace that binding's model-visible replay exactly once. The newest valid checkpoint is the reconstruction root for its named binding; its source Anchor remains provenance. Recovery starts with the checkpoint's replay contract, synthetic body, and inline retained-group items, then applies only non-duplicating same-binding successor-epoch delta suffixes through the newest matching Anchor. A checkpoint without a later accepted request is sufficient to reconstruct context without a successor Anchor; any later accepted request lacking a completed matching outcome and Anchor retains the existing uncertain-request read-only behavior. The 64-MiB and 4,096-item replay-prefix bounds apply to that reconstructed root plus successor-epoch deltas, not the summarized historical prefix. Recovery rejects a gap, duplicate, regression, direct cross-binding checkpoint application, a record appended after the checkpoint that names its superseded context epoch, a same-binding successor delta carrying another replay contract, or a replay delta duplicating or crossing the checkpoint boundary. Historical records at or before the source boundary remain valid and byte-identical. A later binding transition inherits the Session's current context epoch unchanged and MUST seed from the newest executable reconstruction: either a successor Anchor whose lineage includes the checkpoint root and later delta suffixes, or the checkpoint itself when no later request was accepted. Naming the checkpoint in `source_checkpoint_sequence` is transition-source evidence, not cross-binding application of its old binding epoch. The new binding's first replay delta carries that binding's replay contract and MUST NOT restore the checkpoint's summarized prefix. Only another valid checkpoint increments context epoch.

This pre-release replacement preserves `format: anchored-session`, the physical envelope, discovery object, checksum representation, and checksum preimage. The current reader accepts every preceding record except the displaced `context_compaction_handoff`, rejects that old kind and every mixed old/new compaction graph, and admits the new checkpoint and context-epoch fields only under their closed shape. A preceding reader rejects the new record or field. Existing history that contains no displaced handoff remains byte-identical; no migration, dual write, omission, downgrade path, or compatibility shim is provided.

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
authority. Keeping content and refusal separate preserves the visible fields
required for exact Chat Completions replay, while the assistant-only invariant
prevents another API dialect from silently assigning refusal meaning to a user,
developer, or system message.
