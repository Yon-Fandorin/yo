---
schema: methexis.knowledge/v1alpha1
id: agent.persistence.format-compatibility
kind: decision
owner: agent-runtime
sources:
  - id: agent.persistence-001
    revision: sha256:49893e382fcc289fd8effe783a408ea9d6b5ae013bbdb737353d7fb74caae8a6
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
first public release, this revision replaces the immediately preceding
string-input semantic `/v1` with the closed structured-input semantic `/v1`
defined below. Its exact shape and UUIDv7 Session identity are part of the
baseline; a matching schema tag alone MUST NOT admit a record.

Every semantic `/v1` commit, including a descriptor-only commit, MUST contain
the exact top-level discriminator `format: structured-input`. A missing or
unknown value, or a Session history containing mixed format generations, MUST
fail closed before semantic admission.

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
`format: structured-input` discriminator, including the string-input semantic `/v1`, the
summary-less physical `/v1` shape, and the development-only semantic meanings named
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

## Rationale

Reusing `v1` before the first release gives the public contract an honest
starting point without preserving experimental numbering. Naming the displaced
development schemas and making closed shape admission part of the baseline
prevents an old record with the same tag from being mistaken for current data.
Persisting SubmissionId and typed reference occurrences at capture time lets
replay recover the accepted input without guessing identity from display text.
One shared policy owner keeps physical and semantic compatibility rules aligned,
while the existing envelope checksum protects the new semantic payload without
creating a second authority.
