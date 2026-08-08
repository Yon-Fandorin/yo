---
schema: methexis.knowledge/v1alpha1
id: agent.storage.session-repository
kind: decision
owner: agent-runtime
sources:
  - id: agent.storage-001
    revision: sha256:840dce68123f20bb5139961b16970179300e5f8a5c81e76584f13a8b588d54b7
relations:
  depends_on:
    - agent.persistence.format-compatibility
    - agent.runtime.command-event-boundary
    - agent.runtime.session-turn-activity
---
# Session repository and capacity

## Statement

A storage-neutral Session Repository MUST own the lifecycle of durable Session
records without exposing files, SQLite, or another physical layout as the
frontend contract. Semantic Session Journal records and Request Audit data are
logically distinct record domains inside that ownership boundary; they are not
independent Session authorities. The initial product MUST NOT introduce a
separate Request Audit repository interface or a generic append-log
abstraction. The current product MUST provide a local implementation and MUST
leave remote storage and any later physical separation of Request Audit data
as evidence-gated implementations. Replication, dual-write, and conflict
resolution are not part of the local implementation.

The first local implementation MUST use one append-only, explicitly versioned
JSON Lines log per Session. JSONL MUST remain a replaceable implementation
detail behind the repository interface rather than a frontend contract.
One semantic commit MUST be encoded as one physical repository envelope. A
command with zero or more resulting events and a batch of observation events
MUST NOT become partially durable through separate physical appends.
`JournalSequence` MUST express semantic replay order, while
`RepositorySequence` MUST express physical append order; neither sequence MAY
be inferred from the other.

Every newly written physical record MUST carry a versioned CRC32C over an
explicit preimage containing its schema, Session identity,
`RepositorySequence`, record kind, exact payload bytes, and the complete
discovery object required by the format-compatibility contract. The repository
writer MUST assign the discovery timestamp immediately before append and MUST
derive the descriptor, optional binding epoch, and optional latest valid
Continuation Anchor `JournalSequence` from the semantic prefix being committed.
It MUST write that summary in the same checksummed envelope; the timestamp
becomes durable only when the envelope does. Recovery MUST read
older records only when the format-compatibility contract explicitly supports
them and MUST validate checksummed records
before admitting them. The checksum MUST NOT be calculated by recursively
serializing a record that already contains its checksum.

The repository boundary MUST provide a read-only discovery port that locates
and validates the last complete envelope of each Session through a bounded tail
read and returns storage-neutral discovery summaries. Opening or using this port
MUST NOT acquire the writer lease, create repository storage, repair records, or
expose JSONL paths. It MAY use an independent read lock only to distinguish an
active writer from an abandoned pending marker; that lock is not a writer lease.

The two logical record domains therefore share one physical availability
boundary and one capacity ceiling in this implementation. It is the initial durable home for bounded, payload-free Request correlation
records and the separate bounded, payload-bearing `model_replay_delta` semantic
record. Replay is Session meaning rather than Request detail and MUST share the
completed Turn, outcome, and Anchor's atomic physical envelope. Durable Request
detail MUST NOT be admitted until a redaction-before-admission contract has an
implemented gate; until then detail remains process-local and volatile.
Independent Request-detail retention or eviction is not part of the first
implementation.
Recovery MUST stream complete lines, MUST treat an incomplete final line as an
uncommitted tail, MUST report corruption in any complete line, and MUST NOT
materialize the entire log merely to return a bounded suffix.
The local implementation MUST allow multiple processes to open the same
repository root and MUST allow different Sessions in that root to have live
writers concurrently. It MUST resolve the root to a stable absolute location
when opened. During migration from the legacy root-exclusive writer, every new
writer-capable repository instance MUST retain a shared compatibility guard on
the legacy writer-lock file for its lifetime. New writer-capable instances MAY share that guard with one
another, but opening MUST fail while a live legacy instance holds the old
exclusive guard, and the shared guard MUST prevent a legacy instance from
opening after a new instance. This compatibility guard is not the root append
coordinator, MUST NOT serialize new writers, and MUST NOT be acquired by the
read-only discovery port. Each Session MUST allow only
one writer owner, acquired before
loading or repairing that Session state and retained for that writer's
lifetime. Failure to acquire the exact Session lease MUST NOT block opening or
writing another Session.

Every physical append MUST be guarded by a durable marker belonging only to
that Session. A reader that observes a marker MUST test the corresponding
Session lease without creating storage: a live owner makes the marker an
in-flight append and the reader stops at the preceding complete envelope; an
unowned marker quarantines only that Session. If rollback cannot be confirmed,
the marker MUST remain and later readers MUST quarantine the Session log rather
than replay an ambiguous complete line.

The configured capacity ceiling remains repository-wide. Writers MUST acquire
a short-lived root append coordinator only around the final repository-size
check, marker publication, physical append and synchronization, rollback when
needed, and marker removal. This coordinator MUST NOT be retained between
appends and MUST NOT prevent another process from opening the root or working
on a different Session. Lock ordering MUST acquire the Session writer lease
before the root append coordinator and MUST never acquire them in the reverse
order. Lock and marker files do not consume the configured record capacity.
When an owner reopens a non-empty Session or recovers after an initial
Session-state load failure, it MUST require a complete snapshot before
accepting another incremental record because it cannot prove that no
in-memory-only gap preceded the reopen or recovery.
Message and tool-output segments present in semantic commits MUST remain
content persistence detail rather than another Session authority.
Compression, indexes, SQLite projections, alternative encodings, group commit,
and a separate Request Audit namespace MUST require measured evidence instead
of being included in the first implementation. Such a later storage split MUST
NOT redefine Session meaning or transfer Session lifecycle coordination.

A successfully persisted semantic commit MUST be published to the in-memory
Journal only after its append and required synchronization complete.
Process-local presentation updates explicitly marked volatile are outside this
durable-before-publication rule and MUST NOT be exposed as durable records. If
semantic work completes but persistence fails, the owner MUST publish the
result as volatile, latch a durable gap, and MUST NOT report that semantic work
as rolled back.

The local repository MUST be enabled by default, restrict its directory and
files to the current user, and provide a configurable capacity ceiling. It
MUST NOT expire records by age or automatically delete Sessions.
When the configured ceiling or the underlying storage prevents another
durable append, existing records MUST remain unchanged, and the active Session
MUST continue in memory without durable appends. The storage owner MUST emit a
typed, persistent storage-pressure notification to every connected frontend
that distinguishes a known cutoff, a known empty log, and an unknown durable
cutoff. A known cutoff MUST carry both the last durable `JournalSequence`,
which MAY be absent when no semantic Journal event is durable, and the last
`RepositorySequence`; neither coordinate may be inferred from the other.
The repository MUST NOT claim a continuous suffix after such a gap. Once
capacity is available again, it MUST publish a complete Session snapshot
before accepting later incremental records as durable.

The first implementation MUST remain a synchronous single-writer path within
each Session. It MUST NOT add a background writer, generic transaction API, or
group commit without measured synchronization latency and append-rate
evidence.


Model replay MUST pass semantic redaction admission before repository append.
Repository capacity and model-context limits are independent: storage capacity
MUST count replay bytes normally, while replay-prefix or model-context exhaustion
MUST complete the Turn as non-resumable without silently truncating, summarizing,
or appending a partial replay chain.

The repository MUST interpret replay presence through the binding's explicit
continuation strategy. A local `exact_replay(local_client)` binding reconstructs
the validated model-visible prefix from its replay-delta chain. A
`backend_managed_state` binding persists the payload-free outcome, Anchor, and
backend locator evidence without a replay delta and MUST NOT synthesize one from
Transcript or Request Audit data.

A future managed Session Repository MAY execute `exact_replay(managed_server)`
using the same semantic Journal and replay chain. Before it can be advertised,
the implementation MUST verify its server and repository identity, selected
replay boundary, replay-content and contract digests, binding epoch,
availability, and retention. Remote storage, replication, and conflict handling
remain deferred and MUST NOT be inferred from the reserved strategy value.

## Rationale

A local-first port supports immediate resume and diagnosis without freezing a
database choice or silently sacrificing old work. The legacy shared guard makes
the lock-granularity migration fail closed across mixed binary versions without
serializing new processes. Session-scoped ownership
allows independent processes without admitting two writers to one semantic
history, while the short append coordinator preserves the exact shared capacity
ceiling. Atomic envelopes, separate semantic and physical sequence spaces, and
checksummed records make partial or corrupted durability explicit.
Durable-first publication, explicit pressure, and snapshot recovery preserve
honest history while transient streaming remains responsive and remote storage
is still future work.
