---
schema: methexis.knowledge/v1alpha1
id: agent.storage.session-repository
kind: decision
owner: agent-runtime
sources:
  - id: agent.storage-001
    revision: sha256:caf1d7de62510009a1c4acb30348792650695cb6d04a3132e25b32e922705d28
relations:
  depends_on:
    - agent.runtime.command-event-boundary
    - agent.runtime.session-turn-activity
---
# Session repository and capacity

## Statement

A storage-neutral Session Repository MUST own durable session records without
exposing files, SQLite, or another physical layout as the frontend contract.
The current product MUST provide a local implementation and MUST leave a
remote repository as a later implementation of the same repository interface.
Replication, dual-write, and conflict resolution are not part of the local
implementation.

The local repository MUST be enabled by default, restrict its directory and
files to the current user, and provide a configurable capacity ceiling. It
MUST NOT expire records by age or automatically delete completed Sessions.
When the configured ceiling or the underlying storage prevents another
durable append, existing records MUST remain unchanged, and the active Session
MUST continue in memory without durable appends. The storage owner MUST emit a
typed, persistent storage-pressure notification to every connected frontend
that identifies the durable cutoff.
The repository MUST NOT claim a continuous suffix after such a gap. Once
capacity is available again, it MUST publish a complete Session snapshot
before accepting later incremental records as durable.

## Rationale

A local-first port supports immediate resume and diagnosis without freezing a
database choice or silently sacrificing old work. Explicit pressure and
checkpoint recovery preserve honest history while remote storage is still
future work.
