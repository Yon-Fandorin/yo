---
schema: methexis.knowledge/v1alpha1
id: agent.storage.session-repository
kind: decision
owner: agent-runtime
sources:
  - id: agent.storage-001
    revision: sha256:4164010fda3703d828f0f52c5dcd104bbaee038518010333f00b91a089ffc086
relations:
  depends_on:
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
Compression, segmentation, indexes, SQLite projections, alternative encodings,
and a separate Request Audit namespace MUST require measured evidence instead
of being included in the first implementation. Such a later storage split MUST
NOT redefine Session meaning or transfer Session lifecycle coordination.

The local repository MUST be enabled by default, restrict its directory and
files to the current user, and provide a configurable capacity ceiling. It
MUST NOT expire records by age or automatically delete Sessions.
When the configured ceiling or the underlying storage prevents another
durable append, existing records MUST remain unchanged, and the active Session
MUST continue in memory without durable appends. The storage owner MUST emit a
typed, persistent storage-pressure notification to every connected frontend
that distinguishes a known sequence, a known empty log, and an unknown durable
cutoff.
The repository MUST NOT claim a continuous suffix after such a gap. Once
capacity is available again, it MUST publish a complete Session snapshot
before accepting later incremental records as durable.

## Rationale

A local-first port supports immediate resume and diagnosis without freezing a
database choice or silently sacrificing old work. Explicit pressure and
checkpoint recovery preserve honest history while remote storage is still
future work. Versioned JSONL makes the first durable bytes inspectable and
stream-recoverable while the repository boundary leaves later storage
optimization open.
