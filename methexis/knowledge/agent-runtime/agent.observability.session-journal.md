---
schema: methexis.knowledge/v1alpha1
id: agent.observability.session-journal
kind: decision
owner: agent-runtime
sources:
  - id: agent.observability-001
    revision: sha256:2f83ce90f51023daa32e5e91ea93f1d82aec2ae5ffed96a6ada0e708fb0aa9c2
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.persistence.format-compatibility
    - agent.runtime.session-turn-activity
    - agent.storage.session-repository
---
# Durable session observation journal

## Statement

One ordered durable Session Journal MUST be the semantic replay source for
Session history and views. A process-local Live Projection MUST own only the
uncommitted tail required for responsive streaming. Backend transport deltas
MUST update that Live Projection immediately without becoming replay authority by
themselves. The Session worker MUST be the sole writer owner. TUI, GUI, and
other frontends MUST consume read-only views that merge the durable prefix with
the live tail by stable item identity.

Exact text MUST be accumulated without interpretation, inserted whitespace, or
other content changes into immutable ordered segments. Agent-message segments
MUST be forced when buffered UTF-8 text reaches 16 KiB, when its oldest
uncommitted byte reaches one second, at a non-text ordering boundary, or when
the message terminates. Tool-output segments MUST use 64 KiB and the same
one-second age, non-text boundary, and termination rules. A size split MUST
preserve valid UTF-8, and joining the segments MUST reproduce the exact original
text. Segment boundaries
are persistence detail and MUST NOT change Chat or Transcript message meaning.

Every message termination observed by the runtime MUST be sealed with a typed
`MessageEnded` outcome of `completed`, `interrupted`, or `failed`. It MUST carry
the segment count and total UTF-8 byte count required to detect an incomplete
reconstruction. When durable append is available, the final non-empty tail and
`MessageEnded` MUST be committed atomically without duplicating the complete
body. When durable append is unavailable, the same final tail and
`MessageEnded` MUST be published atomically as explicitly volatile Live
Projection state. That volatile terminal seal MUST NOT become replay authority
or a Continuation Anchor. A later complete Session snapshot MUST include the
sealed message and becomes authoritative only after durable publication.
During recovery, a durably recorded message with no terminal record MUST be
sealed as interrupted before later durable events are accepted and MUST remain
visibly partial rather than becoming a completed message.

The Journal MUST also record backend-neutral semantic Session, Turn, and
Activity events together with bounded, payload-free Request correlation and
availability records, plus a separate bounded `model_replay_delta` containing
the exact model-visible replay suffix. Stable operation identity,
accepted-request identity, correlated resumable outcome, backend kind and
version, observation boundary, exchange kind and direction, payload schema
identity, and the versioned backend Session locator required for continuation
belong to those Journal records; they are not Request detail. Requests, responses,
notifications, server-initiated requests, retries, and terminal outcomes MUST
remain distinguishable through correlation records even when detail is
unavailable.

Semantic meaning MUST be committed at capture time and MUST NOT later be
reconstructed solely by interpreting an old backend wire payload. A
backend-specific Request Audit detail, including request payloads, headers, and
revision or attempt evidence, is a logically distinct optional diagnostic
domain under the same Session Repository lifecycle. It MUST NOT become
semantic authority, and its absence MUST NOT block Journal replay or
Continuation Anchor validation. Missing, unsupported, volatile, or unpersisted
detail MUST remain explicit rather than blocking unrelated semantic records.
Redaction MUST happen before any durable Request-detail admission. Credentials,
complete environment variables, private reasoning values, and other prohibited
raw values MUST NOT enter durable Request Audit storage; removal MUST be
represented explicitly when it affects interpretation. A schema-bound
provider-private replay item is not Request detail and MAY enter the semantic
Journal only when the selected binding, Connector, backend, continuation, and
Session Repository contracts jointly define its source, lossless validation,
byte bounds, binding epoch, durable schema, request projection, and exclusion
from every frontend and diagnostic projection. Until the Request-detail
admission boundary is implemented, Request detail MUST remain process-local and
volatile.


A model replay delta MUST preserve exact visible message roles and bytes, validated
function calls, bounded function results, and their stable order. It MUST be
committed after `TurnFinished(completed)` and before the correlated resumable
outcome and Continuation Anchor in the same physical append. It MUST NOT be
derived from Chat or Transcript presentation. Tool arguments and outputs MUST
pass semantic redaction admission before they update an Activity, become later
model input, or enter replay; the admitted exact replacement is authoritative.
An admitted provider-private item MUST remain payload-bearing semantic replay,
MUST be committed atomically with its adjacent visible replay, and MUST NOT be
copied into a message segment, Activity, correlation record, Live Projection,
Transcript, Request Audit, discovery summary, error, log, or diagnostic.

## Rationale

Separating transient presentation from durable semantic replay preserves
responsive streaming without turning arbitrary backend chunks into the Session
contract. Bounded immutable segments limit crash loss and record size, while a
terminal seal distinguishes complete output from a recoverable partial
message. Stable meaning and bounded correlation remain backend-independent as
optional Request Audit detail evolves.

## Nonsecret interview capture and editable storage

At receipt of an admitted nonsecret interview batch, the Session worker MUST
capture its complete backend-neutral question schema as semantic Journal data,
including original order, stable interview identity, batch revision and the
Session, Turn and Activity correlation required by live responses. Question
ids, prompts, ordered options, notes admission and nonsecret classification
MUST be captured before later navigation or recovery needs them. Accepted
answers MUST be recorded as semantic observations in their captured order.
Backend wire request ids MUST remain adapter-private; old wire payloads,
Request Audit and rendered current-question snapshots MUST NOT become the
source of missing interview meaning.

An encoded complete batch record MUST be bounded at 1 MiB. The first excess
byte or a secret question MUST make persistent recovery explicitly unavailable
without widening the existing live-interview admission or silently truncating
the batch. A legacy Journal without complete capture MUST remain replayable
under its existing schema and MUST NOT claim full interview recovery. Volatile
capture MUST remain explicitly volatile and MUST NOT become a durable recovery
promise or Continuation Anchor.

Unsubmitted drafts are editable user state, not semantic Session history. A
separate typed local working-copy repository MUST own that storage; it MUST
NOT append each edit to the Session Journal or confer replay, continuation or
approval authority. The TUI working-copy controller is its sole writer for
that copy; the Session worker remains the sole Journal writer. The repository
MUST store only bounded public interview identity and revision, ordered
per-question drafts, selected option identities, notes, navigation state,
explicit user-entered additional context and submission bookkeeping. It MUST
NOT store credentials, backend RPC identities, secret-marked values or
provider-private reasoning/replay payloads. The complete encoded working-copy
record MUST be bounded at 256 KiB; excess MUST preserve the current in-memory
copy and expose persistence as unavailable.

The working-copy directory MUST be current-user-owned with mode 0700; data and
same-directory temporary files MUST be current-user-owned regular files with
mode 0600. Admission MUST reject symbolic links, unexpected ownership,
unsupported schema and invalid or oversized records without automatic deletion
or migration. Publication MUST use an exclusively created same-directory
temporary file, a complete bounded write, file synchronization, atomic rename
and parent-directory synchronization. Revision compare-and-swap under an
exclusive repository lease MUST prevent concurrent copies from overwriting a
newer saved revision. Conflict or storage failure MUST preserve both the
winning durable record and the caller's editable copy, expose the failure and
MUST NOT claim the caller's revision was saved. Owned abandoned temporary
files MUST be cleaned without deleting another writer's data.

The controller MUST attempt durable save no later than one second after an
edit and at question navigation, explicit submission and graceful application
exit. Recovery promises the last completed durable publication, not unsaved
keystrokes after a crash. Successful live submission bookkeeping MUST bind the exact final
Activity-response request, successful response write and observed response
Activity; new-conversation bookkeeping alone binds its accepted initial StartTurn
request and MUST NOT delete or rewrite the original
submitted answers. Reopening MUST create a distinct editable copy referencing
the immutable captured batch; recovered data MUST remain editable before any
explicit new request. Missing or unreadable storage MUST not block unrelated
Session history or live nonsecret interview use.

## Admitted snapshot and working-copy profiles

Interview capture MUST use the existing typed user-input Activity boundary and
Journal grammar, not a new record kind or an extension to
`yo.semantic-journal-commit/v1`. The first genuine `UserInputRequest` Activity
MUST carry the complete batch in its authoritative TextSnapshot. Later genuine
question request Activities MUST carry the question variant below. The genuine
final `UserInputResponse` Activity MUST carry the accepted-answer variant only
after the exact final response write succeeds. These use existing
`ActivityStarted(UserInputRequest { request_id })` or
`ActivityStarted(UserInputResponse { request_id })`, `ActivityUpdated(TextSnapshot)`
and existing Activity termination events, through `event_committed` and the
existing message-segment/reset/end normalization. The initial complete batch
and request admission MUST be one physical semantic commit under the existing
repository rules. No synthetic ModelWork, agent message or capture Activity
MUST be introduced. Existing activity lifecycle rules remain unchanged.

Capture interpretation MUST require the enclosing typed Activity kind, not a
text marker alone. Batch and question variants require a genuine
UserInputRequest and its matching core ActivityRequestRef; an accepted-answer
variant requires the actual matching UserInputResponse, original committed
RespondToActivity command and successful response completion evidence. The
worker MUST validate these typed correlations before durable admission and
again during recovery. Identical profile bytes in ModelWork, AgentMessage,
tool output or any other Activity MUST remain literal content and MUST NOT
produce an interview capture. The request/response Activity discriminator is
already part of the accepted closed persistence grammar in
`crates/yo-core/src/journal/codec/wire/event.rs`; its existing reference grammar
is in `codec/wire/identity.rs`. No new event, Activity kind or Journal field is
added. Profile values are backend-neutral request/response semantics admitted
at capture time, not saved backend payloads or ordinary model text.

Frontends MUST show the readable question or accepted answers from these
request/response snapshots, preserve their actual question/answer provenance
and MUST NOT expose backend wire ids or treat captures as model replay. Older
readers retain genuine input request/response history under the unchanged outer
Journal grammar; they MUST NOT claim full recovery for an unsupported snapshot
profile. Existing literal model text remains byte-identical. The existing outer
Journal/envelope grammar, sequence allocation, checksums and bounds remain
unchanged.

The snapshot profile `yo.interview-capture/v1` is a closed object with exactly
`schema`, `capture` in that writer order. `schema` is that exact profile string.
The closed `capture` object is exactly one of these variants:

- A batch: `kind: batch`, `interview`, `revision`, `questions`,
  `current_question_id` in that order. `questions` MUST be nonempty, and
  `current_question_id` MUST name its first question. The enclosing genuine
  request Activity and request id MUST equal `interview`.
  `interview` is the original first question's core ActivityRequestRef. Each
  ordered question has exactly `id`, `prompt`, `question`, `options`,
  `allow_free_text`, `allow_notes`, `is_secret` in that order. Question ids MUST
  be nonempty and unique. Each ordered option has exactly `id`, `label`,
  `description`; option ids are canonical decimal strings from `1` through the
  option count, assigned from the captured order. Admission flags are JSON
  booleans, and `is_secret` MUST be `false` for every question. `revision` is
  lowercase `sha256:` plus the SHA-256 of the canonical closed object containing
  `interview`, `questions` in that order, excluding the revision itself.
- A later question: `kind: question`, `interview`, `revision`, `question` in
  that order. `question` has the exact batch question shape above and MUST
  match one question in that immutable captured batch. The original root
  interview request and enclosing current question request MUST share the
  Session/Turn. The worker's live grouping and the admitted snapshot together
  record that membership at capture time; recovery MUST NOT reconstruct it
  from backend wire or Audit. Missing complete batch data leaves only the
  readable current question and makes full recovery explicitly unavailable.
- Accepted answers: `kind: accepted_answers`, `interview`, `revision`, `answers`,
  `answer_responses`, `final_request`, `response_activity` in that order. Answers
  MUST have exactly `question_id`, `option_id`, `text`, `notes`; the array MUST
  contain each captured question once in the original order. `option_id` is null
  or an exact captured option id. Free text and notes MUST satisfy their captured
  admission flags. `answer_responses` is an equally ordered array with exactly
  `question_id`, `request`, `response_activity` for each answer. Its `request` is
  the genuine question's core ActivityRequestRef; its `response_activity` is the
  genuine completed UserInputResponse correlated to that exact committed
  RespondToActivity answering command. Each request MUST resolve to that exact
  question in the same captured batch and original Session/Turn. For every
  answer, admission and recovery MUST compare all option, free-text and notes
  values with the backend-neutral answer projection of that exact committed
  input, using the captured option identities and admission flags. No value may
  be supplied only by the final snapshot, editable copy or backend wire payload.
  When a question was answered again, the entry MUST reference its most recent
  successful committed answering response before final submission, not an older
  answer or a PreviousQuestion/navigation command.
  `final_request` and `response_activity` name the exact successfully written
  final live Activity response and its observed response Activity. The enclosing
  typed UserInputResponse and request id MUST match those identities. The final
  answer_responses entry MUST equal that final request/response pair. All answer
  correlations MUST be revalidated, not only the final question's; missing,
  mismatched or incomplete evidence MUST reject complete accepted capture.
  Earlier completed answering Activities remain partial interview observations
  until the final backend response write succeeds. Navigation, partial local
  staging or a failed final write MUST NOT create this complete final seal.

An ActivityRef is the existing closed `{turn, activity_id}` object; `turn` is
exactly `{session_id, turn_id}`. Session ids are canonical UUIDv7 strings;
Turn, Activity and Request integers are positive u64 values. An
ActivityRequestRef is exactly `{activity, request_id}` with that ActivityRef.
These are core identities, never backend JSON-RPC ids. Canonical writer field
order follows these declarations recursively. Strings are exact Unicode UTF-8.
JSON encoding MUST be compact without whitespace or a trailing newline, with
quote and backslash escaped, control shortcuts `\b`, `\t`, `\n`, `\f`, `\r`,
and every other U+0000–U+001F character as lowercase `\u00xx`; other Unicode and
slash remain unescaped. Booleans and null use JSON literals; integers use
unsigned decimal without leading zeroes. Duplicate or unknown fields, invalid
UTF-8, invalid ids, wrong types, ordering mismatches and unrecognized profiles
MUST reject interview interpretation. Older readers may retain the admitted
TextSnapshot as literal history but MUST NOT claim supported interview recovery.
The 1-MiB limit is measured on these complete canonical snapshot UTF-8 bytes,
before enclosing Journal JSON escaping; existing physical bounds still apply.

The separate working-copy profile `yo.interview-working-copy/v1` is a closed
object with exactly these fields in writer order: `schema`, `copy_id`,
`generation`, `source`, `answers`, `current_question_id`, `context`, `submission`.
`schema` is that exact profile string. `copy_id` is a canonical UUIDv4 string
allocated once for a copy and used as its safe repository locator.
`generation` is a positive u64 mutable CAS generation independent from batch
revision. Each publication increments it exactly once under the lease; overflow
MUST fail without changing the saved record. `source` is exactly `{interview,
revision}` and MUST resolve to a complete admitted capture. `answers` has the
same closed ordered answer shape above, but permits empty unsubmitted values.
`current_question_id` names one captured question. `context` is exact explicit
user-entered UTF-8. Every newly reopened copy MUST get a new copy_id and initial
generation 1, even when its source and answer bytes equal a submitted copy.
The preceding copy and its submission marker MUST remain immutable under that
reopening action. Canonical encoding uses the same rules above; the 256-KiB
limit covers the entire canonical working-copy UTF-8 file, not decoded text or
only answers. Readers MUST reject duplicate/unknown fields and malformed,
inconsistent, oversized or unsupported records before admitting them.

`submission` is null or exactly one closed variant. The `activity_response`
variant has `kind`, `final_request`, `response_activity` in that order and names
the matching final accepted-answer capture and successful response evidence;
it MUST NOT require or manufacture `backend_request_accepted` for
RespondToActivity. The `new_conversation` variant has `kind`, `turn`,
`submission_id`, `accepted_request_sequence` in that order, with the new
Session's TurnRef, canonical UUIDv4 SubmissionId and the positive
JournalSequence of the matching durably observed `backend_request_accepted`
for that exact initial StartTurn. Only that latter path uses accepted model
request identity. Both markers record submission, not Turn completion or
successful execution; absent/volatile acceptance MUST remain unconfirmed and
MUST NOT cause automatic retransmission.

## Secret interview capture and redacted storage profiles

The nonsecret v1 profiles above retain their exact bytes and meanings. A batch
containing at least one secret question MUST instead use
`yo.interview-capture/v2`; its working copy MUST use
`yo.interview-working-copy/v2`. A v2 batch and later-question capture retains the
v1 field order and bounds. Public questions retain the v1 question shape. A
secret question has the same shape but MUST set `is_secret: true`,
`allow_free_text: true`, `allow_notes: false` and `options: []`. A batch with no
secret question is noncanonical under v2. A secret question MUST have a valid
typed secret presentation before its request Activity is admitted; failure MUST
NOT degrade to an ordinary text presentation.

The v2 `accepted_answers` variant retains the v1 fields and correlations, but
each ordered answer is exactly one of two closed shapes. A public answer is the
unchanged v1 object `{question_id, option_id, text, notes}`. A secret answer is
exactly `{question_id, secret: submitted}` in that writer order. It contains no
value, hash, byte count, option, text or notes. Its corresponding
`answer_responses` entry MUST identify the latest successful committed
payload-free secret-input receipt for the exact secret question request and its
completed UserInputResponse Activity. Recovery validates that correlation
instead of comparing the unavailable answer bytes. Intermediate receipts remain
partial observations; only the successful final aggregate response write and
the existing final seal make the ordered batch submitted. A transport attempt
that fails or becomes disconnected MUST NOT create that seal or authorize a
retry, even if delivery may have occurred.

The v2 working-copy object retains the v1 top-level field order, publication
rules and 256-KiB whole-record bound. Its ordered public answers retain the v1
shape. Each secret row is exactly `{question_id, secret: reentry_required}` and
has no other fields. Encoding MUST be unable to accept a secret value, including
when no capture catalog is available. The marker is public editable-state
metadata, not an answer and not evidence of non-delivery. A v2 copy permits null
submission or the existing `activity_response` submission evidence; it MUST
reject `new_conversation`. Preview and export render only a fixed re-entry
notice for the secret row. The ordinary StartTurn new-conversation dispatcher
MUST reject every v2 copy and MUST NOT synthesize empty, masked or explanatory
answer text.

A live secret value is a non-serializable response payload with redacted debug
presentation. After the exact backend command returns successfully, whether it only stages an
intermediate answer or performs the final aggregate response write, the Session
runtime MUST replace the live command value with the payload-free secret-input
receipt before engine commit, Journal append, snapshot creation or any frontend
event. The persistence codec
MUST reject the live form. The backend adapter MAY retain each earlier secret value only in process memory,
bound to its originating ActivityRequestRef and current secret-containing batch,
including an all-secret batch, after that individual request completes and until
the final aggregate response attempt. A newly dispatched live secret response
still requires its exact request to be outstanding. Navigation back to a secret
question removes its previous retained value and requires a new live receipt. Final write success, failure, cancellation and
Turn termination discard all retained secret values. A Yo diagnostic MUST use
static public wording after secret dispatch and MUST NOT include backend stderr
or a backend failure string that could echo the value.

Yo MUST NOT directly copy the entered secret value into its Journal, Request
Audit, message or Activity text, Live Projection, capture/catalog, working copy,
preview, transcript, chat, export, log or diagnostic. This restriction does not
claim control over intentional backend delivery, backend/provider retention,
later model/tool output, clipboard ownership, swap, process memory or crash
dumps.
