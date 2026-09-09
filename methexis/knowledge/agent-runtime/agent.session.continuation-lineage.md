---
schema: methexis.knowledge/v1alpha1
id: agent.session.continuation-lineage
kind: decision
owner: agent-runtime
sources:
  - id: agent.session-001
    revision: sha256:a48404355f43edd4508b79a881fd01776ab02ef482ea8384f31385c424be92ad
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.observability.session-journal
    - agent.runtime.session-turn-activity
    - agent.storage.session-repository
---
# Session continuation and lineage

## Statement

A Yo Session is the stable, durable identity of one user task. It MUST use a
UUIDv7 and MUST remain the same when its Agent Backend, backend Session locator,
transport, or model changes. Those replaceable execution details belong to
ordered, versioned backend binding epochs recorded inside the Yo Session. Every
replacement binding transition MUST close the previous epoch before opening the next, and
every Continuation Anchor MUST identify its epoch. Journal consumers MUST
preserve epoch boundaries and MUST NOT replay, summarize, or attribute backend
state as though one binding spanned a transition.

Only an intentional user fork creates a new Yo Session. A fork MUST record its
parent and either its source anchor or the explicit absence of one. An empty
child offered when no durable anchor exists is such a fork, not a backend
reconnection. A non-empty fork MUST seed its first binding from the source
anchor, checkpoint-only or initial-fork reconstruction through a verified backend-native fork or through the same exact-replay
and explicitly approved lossy-handoff rules used for replacement bindings.

History viewing and executable continuation MUST remain separate capabilities.
A Continuation Anchor MUST identify an accepted backend request, its correlated
stable resumable outcome, the fully committed semantic Journal boundary, and
the versioned backend binding and locator needed to continue. Those identities
and the locator are bounded Session Journal correlation data, not optional
Request Audit detail. Without a context checkpoint or complete initial fork seed, resume MUST select the
newest durable Continuation Anchor and MUST NOT fall back to an older binding
locator. With a valid checkpoint, resume MUST select that checkpoint as the
model-context reconstruction root, then apply only successor-epoch replay
through the newest successor-epoch Anchor; its source Anchor is provenance, not
the current reconstruction boundary. An older backend lacks the later committed
history and may only become a replacement binding through the replay rules
below. Incomplete, unaccepted, or uncommitted suffixes MUST remain diagnostic
evidence and MUST NOT become automatic continuation input.
Request payloads, headers, revision or attempt evidence, and other Request Audit
detail MUST NOT be required to construct or validate an Anchor.

When no durable Continuation Anchor, valid context checkpoint or complete
initial fork seed exists, yo MUST open the saved Session read-only. A checkpoint with no later
accepted request may reconstruct its successor context without a
successor-epoch Anchor. A later accepted request without a completed matching
Anchor remains uncertain and MUST open read-only rather than be resent. Yo MAY
offer an explicitly confirmed empty child Session that
records its parent and the absence of a source anchor, but it MUST NOT replay
or resend the uncommitted suffix. A recovery snapshot MAY support a later
Continuation Anchor only after durable publication completes and every anchor
condition is satisfied; the snapshot alone MUST NOT create one.

Every backend binding MUST declare exactly one versioned continuation strategy.
`exact_replay` MUST declare an executor of `local_client` or `managed_server`;
`backend_managed_state` MUST NOT declare a replay executor. Strategy is an
explicit binding capability and MUST NOT be inferred from backend kind, Provider,
API dialect, or model name. It is distinct from the binding-transition mode:
for example, a newly opened binding can be seeded by an `exact_replay` transition
and then continue using either declared strategy. An exact-replay binding MUST
carry the complete effective binding's `replay_profile` in its binding evidence.
`semantic-only/v1` forbids private replay; `kimi-private-local-plaintext/v1`
declares `kimi.assistant-message/v1alpha1`. The profile is part of binding
identity and epoch freshness and MUST NOT be inferred from ModelId. The
format-compatibility contract's exact legacy omission decodes only as
`semantic-only/v1`.

Both exact-replay executors share one semantic replay contract, validation, and
Anchor boundary. The executor changes only where the validated prefix is loaded
and the next model request is assembled. `local_client` reconstructs that prefix
from the local Session Repository. `managed_server` reserves the same operation
for a future Yo-managed Session service; it MUST NOT be advertised until the
remote repository identity, replay boundary, content and contract digests,
binding epoch, availability, and retention are verified by an independently
reviewed implementation.

A backend using `backend_managed_state` MUST reconnect through the anchor's
versioned locator and verify the returned backend identity. Successful backend-managed resume continues the same Yo Session and binding. One `backend_native_model_rebind/v1alpha1` transition MAY preserve host-owned conversational state only for the same delegated Host and verified authenticated account when the live protocol creates a distinct candidate locator through a state-preserving native fork, advertises model mutation for that candidate, and confirms the exact applied model afterward. The source locator MUST NOT be mutated or reused by the new binding. Yo fully prepares and verifies the candidate while the source epoch remains current, then atomically closes the source epoch and opens a new epoch naming the candidate locator and confirmed model identity without claiming semantic replay, cache restoration, or cross-account portability. Unsupported fork or mutation, account drift, missing confirmation, a mismatched model, or publication failure leaves the source epoch and locator unchanged and executable; the unbound candidate is discarded or quarantined and never becomes continuation authority.
Yo still owns the durable transcript, semantic events, correlation evidence, and
locator, while the backend owns the model-visible conversational state. Such an
Anchor MUST NOT claim or reference a Yo replay delta. If continuation under the
recorded strategy fails but a
replacement backend supports exact semantic replay, yo MAY create a new backend
binding inside the same Yo Session and seed it from the committed semantic
boundary. Exact semantic replay preserves message roles and order, exact
committed text, tool-call and tool-result relationships, and every other
backend-visible semantic record required by the target adapter; it does not
claim to restore provider caches or identical future output. A binding whose
replay profile declares provider-private replay MUST additionally preserve every
required private item losslessly through the same Anchor and atomic replay
commit. That item is eligible only when the resumed binding has the same exact
binding identity and replay profile; it MUST NOT be projected as generic
history. If the source Anchor covers any private item and the target binding or
profile cannot consume that exact item, the transition is never `exact_replay`:
it requires the separately approved `lossy_handoff` path unless an independently
reviewed lossless conversion contract exists. The same rule covers K3 effort or a K2.7 Code ModelId or speed-tier change,
as well as any endpoint, connector, replay-profile, or schema change, even when
the target itself does not require private state. Missing, wrong-binding-epoch, unbounded, or wrong-schema
source state likewise makes exact replay unavailable rather than lossy by
omission. The binding transition, backend and model identities, replay boundary,
private-replay availability, and known cache loss MUST be recorded.

The executable source for a replacement binding MUST be the newest complete
reconstruction, never merely the Anchor that predates a checkpoint. When a
successor-epoch Anchor exists, its lineage includes the checkpoint root and
later delta suffixes. When a checkpoint has no later accepted request, the
transition MAY name that checkpoint directly as its source. This direct source
is a binding-transition seed, not application of the old checkpoint inside the
new binding. It replays the checkpoint's contract, synthetic body, and inline
retained groups after target-profile validation. A transition MUST NOT select
the checkpoint's older provenance Anchor and thereby reintroduce its summarized
prefix.

If only a lossy handoff is possible, yo MUST open the saved Session read-only,
describe the missing or transformed context, and ask once before continuing. The concrete `transformed-semantic-handoff/v1alpha1` path admits Host-to-Managed, Managed-to-Host, cross-Host, and non-native same-Host replacement. It projects only the newest fully committed semantic boundary: visible user and assistant text, ordered tool calls and correlated tool results, and required role and ordering metadata. It MUST exclude provider-private reasoning, encrypted payloads, backend caches, hidden backend context, uncertain requests, and uncommitted suffixes. Before confirmation Yo reports the source and target identities, transformed boundary, included semantic classes, excluded loss classes, and whether private state is present without revealing private contents. The accepted target seed and its digest MUST be durable before the source epoch closes; target rejection or any dual failure preserves the source epoch and locator.
Explicit approval MAY create a replacement binding inside the same Yo Session,
but the Journal MUST record a visible context-loss boundary and the original
durable history MUST remain intact. Yo MUST NOT silently perform a lossy
handoff, resend an uncertain request, or describe a replacement binding as
native resume. Provider-private replay contents MUST remain hidden during that
disclosure; the operator sees only its schema, presence, byte count, and whether
the target can preserve it.

Context history MUST use a positive monotonic Session-global `context_epoch` independent of the backend binding epoch. The initial binding starts context epoch 1, and each later binding inherits the Session's current context epoch unchanged. Only a durable same-binding `yo.context-checkpoint/v1alpha1` advances it by exactly one; a context-policy replacement or binding transition does not. Every accepted model request MUST identify both the binding epoch and context epoch current at dispatch. An active Turn may cross a checkpoint only between complete correlated semantic groups and before its next ordinary Turn request; its terminal replay delta, resumable outcome, and Continuation Anchor use the newest epoch and latest accepted request, while earlier requests remain historical evidence in their original epochs. Recovery MUST apply replay deltas only to their exact current context epoch, apply a checkpoint with its exact replay contract and inline retained replay items as the sole atomic replacement that opens its named successor, and reject gaps, duplicates, regressions, direct cross-binding checkpoint application, or records appended after a checkpoint that name its superseded epoch. Historical records at or before the checkpoint source boundary remain valid evidence. A retained provider-private item remains valid across this same-binding context-epoch increment because its `binding_epoch` is unchanged; only a binding-epoch mismatch is cross-binding-epoch private state. The reconstructed replay bound measures the checkpoint's synthetic user-role body, inline retained groups, and non-duplicating later successor-epoch delta suffixes rather than the replaced prefix. Changing context epoch alone MUST NOT close or open a backend binding, claim Provider-native resume, alter binding-transition cache evidence, or infer Provider cache preservation or loss. Only cache-read tokens reported by a later actual ModelWork usage receipt are evidence of a cache read.

Persisting enabled `portable-summary/v1alpha1` in `yo.context-policy/v1alpha1` selects the standing automatic policy for the backend's exact bounded compaction pipeline and does not ask again at each pressure event. Explicit idle `/compact` is the matching manual authorization for that same pipeline. Yo MUST show the resulting lossy boundary, measurements, retained raw budget, receipt count, and loss classes. This is the sole exception to the preceding per-handoff approval rule and covers only the exact source Anchor and semantic boundary, fixed-structure visible summary, retained semantic suffix, Session-scoped artifact receipts, dropped-private disclosure, and successor context epoch atomically committed by the backend compaction contract. Disabled compaction and `exact-replay-only/v1alpha1` permit no automatic or manual loss. Provider, Model, connector, endpoint, replay-profile, schema, or any other replacement-driven lossy handoff MUST still open read-only, describe the loss, and obtain one explicit confirmation; it MUST NOT reuse context-compaction policy or advance only the context epoch.

## Intentional child fork

An unqualified fork selects the newest complete durable reconstruction of an
idle parent. An explicitly selected historical fork MAY instead use an older
complete Anchor, checkpoint-only reconstruction, or initial-fork reconstruction
from one validated durable parent snapshot. The parent MUST still satisfy the
newest-state executable and idle requirements: an active Turn, pending input
admission, outstanding activity request, compaction, or uncertain accepted suffix
rejects a non-empty fork, including a historical fork. This extension does not
authorize branching directly from an unavailable archived parent. The separately
confirmed empty-child recovery path remains independent: it inherits no model
context or history and never resends an uncertain suffix. Ordinary `/new` remains
independent and has no inferred parent.

A child MUST have a fresh UUIDv7, its own writer lease, its own
JournalSequences, and initial binding epoch 1. It MUST persist one immutable
`initial_fork_seed` under `yo.session-fork-seed/v1`, identifying its parent
UUID and either an exact source Anchor, an exact checkpoint-only
reconstruction, a valid initial-fork reconstruction without later accepted
child work within the selected point, or explicit empty origin. A parent
coordinate MUST always be qualified by the parent SessionId. It MUST NOT be
interpreted as a child JournalSequence, a previous child binding, or a child
accepted request. The child's initial fork binding is not a binding
replacement: it has no previous epoch to close.

An Anchor source MUST include its binding/context epochs, exact source record
sequence and fully committed journal boundary. Its seed MUST use the complete
reconstruction at the explicitly selected point, or the newest complete point
when no historical selection was made, including the checkpoint root and all
required deltas effective there. A checkpoint-only source MUST name that
checkpoint itself, not its older provenance Anchor. Each selected reconstruction
MUST be quiescent and contain no accepted request lacking matching completion.
Later committed parent work MUST NOT enter its model seed or inherited history.
Physical envelopes and every discovery summary through the captured parent
cutoff MUST be validated before selection; presentation rows and discovery hints
are not boundary proof. The selected source is immutable once captured. Missing
historical replay or private evidence MUST reject selection; a later summary or
archival transcript MUST NOT reconstruct lost model state. An explicitly
confirmed empty child records source `empty` and inherits no model input.

A complete initial-fork reconstruction is a valid non-empty source only before
the first later accepted request within that reconstruction. Historical selection
MAY select that earlier point after the parent has completed later work, subject
to the newest-parent eligibility requirement. Validate the complete initial seed
and effective binding at the selected point, including any exact replacement and
import ownership chain. A new child copies that reconstructed context and
flattened inherited history. It MUST NOT fall back to a grandparent Anchor, fetch
an ancestor repository, or silently create an empty fork. Its immediate parent
is the selected Session; ultimate item origins remain unchanged.

The interactive historical selection form is exactly `/fork at`; `/fork` retains
newest-point behavior. `/fork at` opens a bounded read-only boundary picker for
the currently selected Session. Other arguments MUST fail locally without input
admission. Choosing a validated boundary explicitly requests child preparation;
dismissal preserves the parent and draft. A stale selection MUST fail rather
than retarget. Session-tree selection retains its existing resume behavior and
does not implicitly fork. No archived CLI or print command is added by this
extension.

The selection handle MUST bind the frozen validated capture, selected logical
cutoff, and effective binding/context ownership, either explicitly or through an
opaque reference to that exact recovered object. A source kind and record number
alone are insufficient. Selection MUST NOT resolve a raw sequence again against
newer recovery state or substitute the newest binding/profile. Historical target
configuration and every required private replay item MUST remain exactly
available; failure rejects preparation before backend work.

Catalog IO MUST enforce finite physical-byte and physical-record limits while
reading the pinned snapshot, before unbounded allocation. If either limit prevents
validation through the full captured cutoff, the entire catalog is unavailable
and no boundary is selectable. Only after full capture validation may a separate
returned-boundary limit produce an explicitly truncated catalog whose verified
entries remain selectable. Catalog viewing creates no backend, Session or writer
lease. The host MUST revalidate the live idle parent and captured cutoff before
preparing the selected child; stale evidence requires a new explicit selection.
All existing whole-seed, replay-item and inherited-history limits still apply;
capacity failure never silently drops required context or history.

An exact-replay child MUST contain the complete validated model-context seed
inline, including the exact system/tool contract, ordered semantic replay items,
and every required eligible provider-private item. It MUST NOT require the
parent's files, repository reader, input-admission service, skill catalog, or
optional supporting assets during recovery. Parent deletion or loss therefore
does not invalidate the child's exact replay. Private item payloads and ultimate source qualifications remain immutable,
while the explicit fork-import mapping assigns them to the child's independent
epoch-1 baseline for runtime validation. Checkpoints, retained groups, exact
binding replacement and repeated forks MUST preserve that mapping and original
bytes according to the persistence profile. Source epochs are provenance, not
values substituted into child epoch-equality checks. The target must preserve
the exact effective binding identity and private replay profile; incompatibility
rejects exact fork rather than silently losing state. The initial fork profile
does not authorize lossy handoff or cross-account/model conversion.

A backend-native child MUST be prepared through a separately advertised fork
capability and obtain a distinct locator owned by the child. Same-host and
verified same-account evidence, the exact target model/binding identity, and
proof that the candidate contains exactly the selected durable boundary are
required. A host's current locator or successful `thread/fork` response alone
is not boundary proof: it can contain later or uncommitted state. The adapter
MUST establish a protocol-supported exact boundary or reject the operation.
It MUST NOT mutate the source locator or advertise model-rebind support as
proof of genuine child-fork support. A native seed records verified native
continuation evidence without inventing Yo semantic replay or cache restoration.

Inherited transcript content MUST be stored as bounded source-qualified history
inside the child seed and displayed with its source Session/boundary. It is
archival presentation evidence only. It MUST NOT be replayed as child commands,
re-admitted as new user input, counted as child model/tool usage, or used to
manufacture child Turns, requests, activity IDs, Anchors or side effects. Forking
a child preserves the original source qualifiers of inherited records rather
than recursively relabeling them as that child's own events. The inline model
seed alone owns inherited model context; presentation history never substitutes
for exact replay. Capacity failure rejects the fork rather than silently
truncating required model input or inherited visible history.

The host MUST prepare the backend candidate and complete child bootstrap while
the parent remains selected with its prior availability unchanged. For a
non-empty fork the parent remains executable; an explicitly confirmed empty
recovery fork preserves the parent's read-only or uncertain state without
claiming it executable. One atomic child publication MUST
contain its descriptor, complete initial fork seed and first binding evidence
before selection can switch. Preparation or publication failure leaves the
parent unchanged; any unbound native candidate and incomplete child are cleaned
up or quarantined and never offered as executable descendants. Recovery MUST
reject a missing, duplicated, late, mismatched or partial seed. Before the
child's first accepted request, its complete initial seed is a valid continuation
root. Afterward, ordinary complete child Anchors and checkpoints supersede it
according to their reconstruction rules; an accepted child request without a
matching completed Anchor remains uncertain and MUST NOT be resent.

A Session tree MUST derive ancestry from validated durable fork provenance,
never matching names, timestamps, backend locator similarity, or model-rebind
epochs. Missing parents are shown as unavailable ancestors. Older histories
without fork provenance have unknown ancestry rather than an invented root.
Tree viewing MUST remain bounded, non-creating and read-only, and must not
start a backend or acquire a Session writer lease. Historical selection follows
the explicit bounded selector above. Tree browsing itself grants neither
executable continuation nor permission to infer or repair a source boundary.

## Image-bearing exact continuation

Exact continuation, exact binding replacement and exact child fork preserve each
admitted input image's immutable canonical PNG bytes, metadata and ordered
text/image occurrence position. The selected target MUST admit the complete
image grammar and accounting policy required by that reconstruction before a
request is dispatched. Unknown image capability rejects image continuation;
an advisory accounting quality is a separate condition and cannot be silently
upgraded to exact evidence. A different target that cannot preserve the complete
input MUST NOT claim exact replay or silently replace images with captions,
paths, thumbnails or omitted parts. Existing separately authorized lossy-handoff
rules remain the only applicable replacement exception.

An exact child seed carries image snapshots inline under the same complete-seed
and replay bounds, including images within retained checkpoint groups. The
existing import mapping keeps ultimate origins unchanged and assigns the child
its independent local ownership; no parent file, repository, image-preparation
service or ancestor access is required after publication. A second-generation
fork must preserve those same bytes after both ancestors and original input
files have been deleted. Source-qualified archived image input remains inert
history, never a new child command or a substitute for the inline model seed.

After compaction, image-loss source coordinates identify the current Session's
actual authoritative replay delta, retained checkpoint group or initial seed
group under the persistence contract. The epoch comes from that named record;
ultimate imported origins are separate provenance. Retained images stay exact
and indivisible. Summarized image occurrences have the new closed image loss
entries and MUST NOT be resurrected from archival history when resuming,
replacing or forking that checkpoint-rooted reconstruction. An explicitly
selected older complete historical point may still contain the original image
if its own exact validated replay retains it; a later summary cannot manufacture
missing earlier input. Existing source-boundary,
latest-parent eligibility, failure-atomic publication and uncertain-request
rules remain unchanged.

## Rationale

The user task should not acquire a new identity merely because its execution
provider changed. Stable Yo Session identity plus explicit binding epochs keeps
that continuity honest, while a durable anchor and visible context-loss
boundary prevent retries, partial writes, cache loss, and backend replacement
from being mistaken for stronger continuation than yo can provide. Explicit
epoch ownership makes the cost of task-level identity visible to every Journal
consumer rather than hiding it behind a Session ID.
