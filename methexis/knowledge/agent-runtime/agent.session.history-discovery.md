---
schema: methexis.knowledge/v1alpha1
id: agent.session.history-discovery
kind: decision
owner: agent-runtime
sources:
  - id: agent.session-002
    revision: sha256:6fdab82cf5b838017ce0f85b17a99430da97b42ed9bc5d310a80bf5f69fd9352
relations:
  depends_on:
    - agent.observability.session-journal
    - agent.observability.view-projections
    - agent.remote.yo-host
    - agent.session.continuation-lineage
    - agent.storage.session-repository
---
# Stored Session discovery and read-only history

## Statement

Stored Session discovery MUST be available without executable continuation. A
new Session MUST durably record a minimal versioned descriptor with its full
UUIDv7 Session ID, workspace-host identity, host-normalized workspace path, and
start time before later Session activity becomes durable. The host that owns
the workspace defines normalization; clients MUST compare the host identity and
that host's normalized path rather than applying local path rules to a remote
path. The descriptor is semantic Session Journal data under the existing
Session Repository lifecycle, not a filesystem index or a second Session
authority. A future compatibility contract MAY admit a descriptor-less format
as readable with explicitly unknown metadata; development formats rejected by
the current compatibility baseline are not readable Sessions.

Every supported physical Session record MUST carry a bounded discovery summary
in the same physical commit. The summary MUST contain the complete Session
descriptor, a writer-assigned `updated_unix_millis`, an optional binding epoch,
and an optional latest valid Continuation Anchor `JournalSequence`. The writer
MUST assign the timestamp immediately before append; it becomes durable only
with the checksummed envelope and is not inferred from filesystem metadata.
The descriptor, binding epoch, and anchor reference MUST be recomputable from
the committed Journal prefix. The summary MUST NOT be written through a second
append or mutable side index and MUST NOT replace the Journal as authority. A
reader obtains current discovery metadata by locating and validating the last
complete envelope through a bounded tail read; it never scans a complete log
merely to list Sessions. “Bounded” limits discovery to the tail envelope rather
than promising that a single valid envelope has a fixed byte size.
The summary is a discovery hint, not semantic proof. Executable continuation
MUST validate the referenced Anchor from the Journal. Any detected disagreement
between summary and Journal MUST treat the Journal as authoritative, report the
discrepancy explicitly, and classify continuation eligibility as `unavailable`
until writer-owned recovery publishes a consistent envelope.

`yo session` and the `yo --resume` picker MUST default to Sessions whose
recorded workspace-host identity and normalized path equal the current
workspace. An explicitly supported descriptor-less Session has unknown
workspace and MUST be reachable through `--all` and direct full-UUID selection,
not inserted into every workspace's default list. `--all` additionally includes
other and unknown workspaces and is the only ordinary list form that displays a
workspace column.
`--details` MUST expose the record schema version, continuation eligibility,
and full recorded path without changing the selected set. `UPDATED` MUST mean
the timestamp of the last valid durable envelope, never volatile screen
activity or filesystem modification time. Results MUST use that timestamp,
then recorded start time and stable Session identity, for deterministic
ordering; unavailable legacy values remain visibly unknown.

Continuation eligibility is durable evidence, not a promise that a backend is
currently reachable. Quarantine or a detected summary disagreement takes
precedence over every summary value. Otherwise it is `eligible` only when the
bounded summary identifies a valid Continuation Anchor by `JournalSequence` in
a supported record schema, `unavailable` when a supported record proves that no
valid anchor exists or the committed prefix is quarantined, and `unknown` when
an older or unsupported format cannot provide bounded evidence. Actual native
resume, replay support, transport reachability,
and lossy-handoff availability are evaluated only by executable continuation.
The picker MUST dim and prevent selection of `unavailable` entries; `unknown`
entries remain inspectable and require continuation-time evaluation rather than
being presented as resumable. Direct `yo --resume SESSION_ID` of an unavailable
Session MUST open its durable history read-only and MAY offer only the explicitly
confirmed fork permitted by the Continuation Anchor contract.

The full UUID is the public Session identifier accepted by `yo session
SESSION_ID`, `yo usage SESSION_ID`, and `yo --resume SESSION_ID`. `yo` without
a continuation option starts a new Session. `yo --continue` selects the most
recently updated `eligible` Session in the current workspace and MUST fail
without creating a Session when no candidate exists.

A stored Session view is an archival Session Repository projection, not the
live frontend view that merges the durable prefix with a process-local tail.
The local read-only CLI grammar MUST consist of these forms:

- `yo session [--all] [--details]` for listing;
- `yo session SESSION_ID [--view chat|transcript|request] [--ascii]` for an
  archived view, defaulting to Chat;
- Transcript alone MAY additionally accept `--limit N`, where N is positive,
  and `--content none|preview|full`; and
- `yo usage SESSION_ID [--ascii]` for the independent Session Usage report.

`--ascii` MUST change glyph selection only. `--limit` and any explicitly
supplied `--content`, including `--content full`, MUST be rejected for Chat and
Request. Any other Session view, including Usage, MUST be a usage error. Usage
MUST NOT be represented by a Session-view enum or route. Both direct-read
commands MUST emit pipeable plain output on stdout and diagnostics on stderr.
A missing or unreadable Session and any fatal projection error MUST retain its
typed local diagnostic and MUST emit no partial stdout.

Both direct-read commands MUST use the local non-creating Session reader and
MUST capture one read-only, point-in-time projection of only the durable
semantic Journal. When the repository can independently establish an active
writer without acquiring its lease, a pending marker is treated as an
in-flight append: the reader MUST stop at the last validated envelope before
that marker and report a durable point-in-time snapshot. When no active writer
can be established, a remaining marker MUST quarantine the Session. Failure to
detect a live writer may conservatively quarantine availability but MUST NOT
admit guarded bytes or weaken snapshot correctness. Neither command may
subscribe to later appends, start an Agent Backend, allocate or resume a
Session, acquire the repository writer lease, create storage, repair a torn
tail, or otherwise mutate repository state. They MUST ignore an incomplete
final line as uncommitted, honor pending-marker quarantine and complete-line
corruption, and preserve explicit interrupted, incomplete, and durability-gap
states instead of presenting a continuous completed history.

The storage-neutral read boundary MUST make listing and replay available
without exposing JSONL paths or write operations to the CLI. It is a read port
implemented by the same Session Repository ownership boundary, not a generic
append-log abstraction, an independent Request Audit repository, or a premature
shared local-and-remote reader interface. Executable resume, backend binding
persistence, native backend reconnection, semantic replay, lossy handoff, and
deliberate fork creation remain outside this capability and continue to require
the Continuation Anchor contract.

## Rationale

Commercial coding agents commonly offer recent-session selection, a session
list, and direct identity selection. Separating bounded discovery and read-only
history from resume lets yo provide the useful inspection primitive required by
every continuation fallback without guessing workspace identity, starting
backend work, or treating an incomplete durable suffix as safe input. A summary
inside each existing durable envelope keeps discovery bounded without creating
a second writer, authority, or recovery path. The closed local command grammar
keeps archived observability and Usage reporting independently addressable
without opening either path into executable continuation.
