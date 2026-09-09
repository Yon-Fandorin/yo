---
schema: methexis.knowledge/v1alpha1
id: agent.session.history-discovery
kind: decision
owner: agent-runtime
sources:
  - id: agent.session-002
    revision: sha256:a37c12fed231112b5fa0bbf8b6b63daf6882be501d1a570cbdcd6818f1077bee
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
an optional latest valid Continuation Anchor `JournalSequence`, and, in the
explicit fork extension, an optional executable initial-fork seed sequence. The writer
MUST assign the timestamp immediately before append; it becomes durable only
with the checksummed envelope and is not inferred from filesystem metadata.
The descriptor, binding epoch, anchor reference and supported initial-seed hint
MUST be recomputable from
the committed Journal prefix. The summary MUST NOT be written through a second
append or mutable side index and MUST NOT replace the Journal as authority. A
reader obtains current discovery metadata by locating and validating the last
complete envelope through a bounded tail read; it never scans a complete log
merely to list Sessions. “Bounded” limits discovery to the tail envelope rather
than promising that a single valid envelope has a fixed byte size.
The summary is a discovery hint, not semantic proof. Executable continuation
MUST validate the hinted reconstruction root from the Journal. Any detected disagreement
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
currently reachable. Quarantine and detected summary disagreement take
precedence. A supported positive Anchor or initial-fork seed hint may establish
`eligible`; absence of a hint in a frozen legacy summary is `unknown`, since it
does not prove the absence of a checkpoint or initial seed. `unavailable` is
reserved for validated absence of executable reconstruction, quarantine or
corruption. The picker MUST dim and prevent selection of unavailable entries;
unknown entries remain inspectable and require continuation-time evaluation.
Direct resume validates an eligible or unknown reconstruction and opens saved
history read-only when validation fails. Only the separately confirmed empty
fork may be offered when no executable source is available.

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

## Initial fork continuation hints

A supported discovery summary MAY additionally contain positive
`initial_fork_seed_journal_sequence`, naming a complete child bootstrap seed.
It is eligible evidence only when the same durable prefix has a valid initial
fork binding and no child accepted request after the seed. The sole writer
MUST omit this hint as soon as a later accepted child request appears; a complete
child Anchor then supplies ordinary continuation evidence. An uncertain later
accepted suffix MUST NOT regain seed eligibility after a failed Turn or restart.
The hint is part of the same checksummed physical envelope, not a second index.

A bounded tail read may classify a supported positive Anchor or valid initial
seed hint as `eligible`, subject to quarantine and summary-consistency precedence.
A frozen legacy summary with neither hint cannot distinguish checkpoint-only,
initial-seed, empty or uncertain histories and is `unknown`, not proof of
unavailability. `unavailable` requires quarantine, established corruption, or
validated evidence that no executable reconstruction exists. Unknown entries
remain inspectable and receive continuation-time evaluation. Listing MUST NOT
scan the full Journal or start a backend merely to upgrade an unknown entry.

Executable direct resume validates the complete initial child seed and binding
when no newer completed child Anchor or checkpoint supersedes it. A valid
checkpoint-only root remains supported under the same continuation contract.
No ancestor repository, skill catalog or backend request audit is fetched to
reconstruct an inline exact seed. Failure or an uncertain accepted suffix opens
history read-only and never resends source or child work. `--continue` may select
an eligible supported initial-seed hint using the same deterministic ordering as
an Anchor hint; it must still pass executable validation before publication or
backend work. Neither inspection nor a positive hint independently grants
executable continuation. Full tree projection and arbitrary historical branch
selection are separate remaining capabilities.

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
