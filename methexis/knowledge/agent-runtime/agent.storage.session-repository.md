---
schema: methexis.knowledge/v1alpha1
id: agent.storage.session-repository
kind: decision
owner: agent-runtime
sources:
  - id: agent.storage-001
    revision: sha256:babaa8fe5a5d034539e75fbc46cf698f639b58a0e744b86b9f088da52873eec6
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
boundary and one capacity ceiling in this implementation. It is the initial
durable home for bounded, payload-free Request correlation records. Durable
Request detail MUST NOT be admitted until a redaction-before-admission contract
has an implemented gate; until then detail remains process-local and volatile.
Independent Request-detail retention or eviction is not part of the first
implementation.
Recovery MUST stream complete lines, MUST treat an incomplete final line as an
uncommitted tail, MUST report corruption in any complete line, and MUST NOT
materialize the entire log merely to return a bounded suffix.
The local implementation MUST allow only one writer owner per repository root.
It MUST resolve that root to a stable absolute location when opened.
Every physical append MUST be guarded by a durable pending marker. If rollback
cannot be confirmed, the marker MUST remain and later readers MUST quarantine
the Session log rather than replay an ambiguous complete line.
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

The first implementation MUST remain a synchronous single-writer path. It MUST
NOT add a background writer, generic transaction API, or group commit without
measured synchronization latency and append-rate evidence.

## Rationale

A local-first port supports immediate resume and diagnosis without freezing a
database choice or silently sacrificing old work. Atomic envelopes, separate
semantic and physical sequence spaces, and checksummed records make partial or
corrupted durability explicit. Durable-first publication, explicit pressure,
and snapshot recovery preserve honest history while transient streaming remains
responsive and remote storage is still future work.
