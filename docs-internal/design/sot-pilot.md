# Methexis SOT Pilot Contract

| Axis | State |
| --- | --- |
| Document | Accepted |
| Implementation | W0 foundation, the approved active five-unit TUI seed, S3 Librarian discovery, and S4 Context Resolution are implemented |
| First corpus | Structured Core `Surface` |
| Product scope | Internal `yo` Pilot |

This document is the tracked authority for the first `tools/methexis` Pilot and
the incubating `tools/librarian`. It owns stable behavior and trust boundaries,
not the exact pre-implementation CLI spelling or serialization details.

Methexis is the product that maintains agreement between approved canonical
knowledge and its Projections. SOT names the architectural authority role and
remains the prefix for stable decision IDs; it is not the product name.

`MUST` and `MUST NOT` are blocking contracts. `SHOULD` requires a documented
reason to deviate. Illustrative paths, commands, and field names are not frozen
public API.

## Decision index

| ID | Owner |
| --- | --- |
| `SOT-001` | Pilot and product boundary |
| `SOT-002` | Knowledge authority and unit model |
| `SOT-003` | Relation and Source model |
| `SOT-004` | Revision, approval, and Checkpoint |
| `SOT-005` | Librarian discovery and location boundary |
| `SOT-006` | Invalidation and validation |
| `SOT-007` | Context resolution and artifacts |
| `SOT-008` | Agent-first interface |
| `SOT-009` | Rust and process boundaries |
| `SOT-010` | Evaluation and graduation |

IDs remain stable even if a decision is later replaced. Downstream design,
Slice contracts, tests, and evidence reference these IDs instead of copying
their rules.

## SOT-001: Pilot and product boundary

`tools/methexis` MUST begin as an internal `yo` Pilot. Its first job is to
improve code-agent work on `yo`; it is not yet a generic knowledge platform.

`yo` is the incubation testbed and first reference consumer, not the expected
permanent owner of Methexis. Repository extraction and domain generalization
are separate gates: validated Pilot capabilities MAY move to a standalone
Methexis repository, while generalizing beyond the `yo`-proven contract
requires evidence from a second real product consumer.

The first evaluation corpus is the Structured Core `Surface` vertical slice. It
SHOULD contain roughly 20–50 units covering geometry, cell width, graphemes,
style, `Surface` invariants, common Inline and Fullscreen output semantics, HTML
projection, fixtures, and validation.

S5 begins with a smaller contract batch for independently reviewable model and
adapter decisions. Implementation Slices add fixture, failure, layout, and
mode-behavior units until the complete Surface corpus reaches the 20–50 range.
The A/B/C evaluation MUST NOT begin while that corpus shape or its executable
evidence is incomplete.

S1 precedes that evaluation corpus with five TUI architecture units that began
as Draft and were later reviewed, approved, and activated. This small seed
exists to exercise the file model, identity algorithm, graph validation, and
agent-facing Fast Check before the larger Surface authoring cost. It is
foundation evidence, not the evaluation corpus or a replacement for the S5
Surface gate.

A small SOT operating-procedure corpus MUST provide a structurally different
secondary sample. It MAY reference the repository workflow authority but MUST
NOT restate or become a second canonical owner for `CONTRIBUTING.md` policy.
Existing workflow rules remain references or generated projections; new
KnowledgeUnits own only SOT-specific procedures not already owned elsewhere.

The domain model MUST NOT contain TUI-specific fields. General domain expansion
beyond the `yo`-proven contract requires a second real non-`yo` product
consumer.

## SOT-002: Knowledge authority and unit model

The knowledge-model clauses in this section are migrating to the following
semantic KnowledgeUnits:

| KnowledgeUnit | Delegated scope |
| --- | --- |
| [`methexis.knowledge.unit`](../../methexis/knowledge/methexis/methexis.knowledge.unit.md) | Meaning of a KnowledgeUnit |
| [`methexis.knowledge.unit-boundary`](../../methexis/knowledge/methexis/methexis.knowledge.unit-boundary.md) | Authoring boundary between one complete contract and independent units |
| [`methexis.knowledge.identity`](../../methexis/knowledge/methexis/methexis.knowledge.identity.md) | Stable semantic KnowledgeId and grammar |
| [`methexis.knowledge.record-format`](../../methexis/knowledge/methexis/methexis.knowledge.record-format.md) | Canonical file and metadata format |
| [`methexis.knowledge.body-contract`](../../methexis/knowledge/methexis/methexis.knowledge.body-contract.md) | Required canonical body structure and visible meaning |
| [`methexis.knowledge.kind-vocabulary`](../../methexis/knowledge/methexis/methexis.knowledge.kind-vocabulary.md) | Meanings of the closed initial kinds |
| [`methexis.knowledge.kind-extension`](../../methexis/knowledge/methexis/methexis.knowledge.kind-extension.md) | Admission gate for new and catch-all kinds |

Until all seven exact approved revisions are selected together by the trusted
active Checkpoint, this section remains the sole authority for those scopes and
the linked records are migration candidates regardless of branch presence or
approval. The migration activation MUST select all seven as one required
closure; partial activation does not transfer any listed scope and MUST be
rejected during activation review. Once that complete activation becomes
trusted, the linked KnowledgeUnits become the sole authority for their listed
scopes and the corresponding prose below remains only non-authoritative
migration history until a later Projection cleanup replaces it with routing
links. This conditional delegation makes the active-Checkpoint transition the
atomic owner change, without an authority gap or a period of dual ownership.

Records reachable from the repository-local `refs/heads/develop` are the only
approval authority in the current Pilot. Task input, environment variables, and
the invoking agent MUST NOT override it. At the start of an operation, the ref
is resolved once to an exact commit; that pinned snapshot is the only authority
used for computation and its commit is recorded in every result. An operation
that promises final authority stability MAY reread only the configured ref and
active-record identities before returning. A mismatch fails the pinned
operation and never switches it to the newer snapshot. An internal injected
policy MAY be used by isolated tests but is not a production input surface.

Authority reads MUST use the system Git executable with caller Git
configuration and environment removed. Replacement refs and graft-like object
substitution MUST be disabled, so the recorded object ID and materialized tree
cannot diverge.

A Task commit, proposed Slice commit, working-tree state, or branch name
supplied by the caller is never authority. Supporting a human-approved Wave
commit as a temporary trust anchor is deferred until repository policy owns a
non-caller-controlled configuration surface.

Knowledge, Source, approval, Checkpoint, and active-Checkpoint records MUST be
tracked. Proposed branch and working-tree edits are Draft inputs until the
repository approval workflow integrates them into the configured trust anchor.
A database or local file MAY be a rebuildable index or cache, but MUST NOT
become a second writable authority. The compiler consumes a storage-neutral
immutable `KnowledgeSnapshot`.

A `KnowledgeUnit` is one independently changeable, approvable, and
invalidatable behavioral contract. It is neither an individual sentence nor a
whole design document. Conditions, outcomes, and exceptions that would be
incomplete alone remain together. Reusable definitions and independently
changing behavior are separate units.

The Pilot uses one Markdown file per unit:

- typed YAML frontmatter for validated metadata;
- a constrained English body for canonical meaning;
- a stable semantic `KnowledgeId`;
- a mutable physical file location.

The Draft corpus begins under:

```text
methexis/
  knowledge/<domain>/<KnowledgeId>.md
  owners/<domain>.yaml
```

The directory and filename are organizational hints, not identity. The loader
MUST read `KnowledgeId` and OwnerId from record content and MUST preserve
identity when a valid record moves. Each ID is lowercase dot-separated semantic
segments. A segment starts with an ASCII letter, ends with an ASCII letter or
digit, and contains only lowercase ASCII letters, digits, or single internal
hyphens. IDs MUST NOT encode the physical path, record kind, revision, or first
consumer.

Frontmatter contains only machine metadata: schema, ID, kind, OwnerId, Source
references, and typed relations. The Markdown body contains canonical meaning;
it MUST NOT duplicate a canonical statement in frontmatter. Every body has a
non-empty `Statement` section. A decision also has `Rationale`; a procedure also
has `Steps` and `Completion Criteria`.

Canonical records MUST NOT use YAML merge keys. They add an alternate metadata
composition mechanism without adding meaning to the closed Pilot schema.
Canonical bodies MUST NOT contain raw HTML blocks or comments; hidden rendered
content cannot satisfy a required semantic section.

The canonical English body is agent-generated and begins as Draft. When Korean
user input is material provenance, a reviewer sees an authorized Source excerpt
and a generated Korean review projection. Full transcripts MUST NOT be retained
by default. Tracked conversation Sources contain only a minimal relevant
excerpt, redact sensitive content, and require explicit human authorization.
Sensitive provenance MAY remain outside Git behind an opaque reference and
content hash. English efficiency is a measured Pilot hypothesis, not a
permanent product assumption.

The closed initial knowledge kinds are:

| Kind | Meaning |
| --- | --- |
| `definition` | A shared term or meaning |
| `rule` | Required behavior, constraint, or invariant |
| `decision` | A selected direction and its rationale |
| `procedure` | Ordered work with a completion condition |

Every kind requires one canonical statement. Procedure additionally requires
steps and completion criteria. Classification friction MUST be recorded before
expanding the enum. Catch-all kinds such as `misc` are prohibited.

## SOT-003: Relation and Source model

The relation, Source, and revision-identity clauses in this section and the
opening of SOT-004 are migrating to the following semantic KnowledgeUnits:

| KnowledgeUnit | Delegated scope |
| --- | --- |
| [`methexis.relation.vocabulary`](../../methexis/knowledge/methexis/methexis.relation.vocabulary.md) | Closed relation names, targets, meanings, and advisory-signal boundary |
| [`methexis.relation.required-graph`](../../methexis/knowledge/methexis/methexis.relation.required-graph.md) | Forward relation authorship, reverse derivation, and graph membership and acyclicity |
| [`methexis.source.kind-vocabulary`](../../methexis/knowledge/methexis/methexis.source.kind-vocabulary.md) | Closed Source kinds, material modes, freshness modes, and current eligibility |
| [`methexis.source.record-format`](../../methexis/knowledge/methexis/methexis.source.record-format.md) | Source record shape, SourceId identity, and kind-specific fields |
| [`methexis.source.revision-identity`](../../methexis/knowledge/methexis/methexis.source.revision-identity.md) | Canonical SourceRevision preimage and exclusions |
| [`methexis.source.reference-pinning`](../../methexis/knowledge/methexis/methexis.source.reference-pinning.md) | Exact Source pins and explicit Source advancement lifecycle |
| [`methexis.knowledge.revision-identity`](../../methexis/knowledge/methexis/methexis.knowledge.revision-identity.md) | Canonical Knowledge RevisionId preimage and line-ending normalization |

Until all seven exact approved revisions are selected together by the trusted
active Checkpoint, SOT-003 and the RevisionId identity paragraphs at the start
of SOT-004 remain the sole authority for these scopes. The linked records are
migration candidates regardless of branch presence or approval. Migration
activation MUST select all seven as one required closure; partial activation
does not transfer any listed scope and MUST be rejected during activation
review. Once that complete activation becomes trusted, the linked
KnowledgeUnits become the sole authority for their listed scopes and the
corresponding prose below remains only non-authoritative migration history
until a later Projection cleanup replaces it with routing links. This
conditional delegation makes the active-Checkpoint transition the atomic owner
change without an authority gap or a period of dual ownership.

The closed initial relation vocabulary is:

| Relation | Target | Compiler meaning |
| --- | --- | --- |
| `depends_on` | KnowledgeUnit | Required for completeness |
| `constrained_by` | KnowledgeUnit | Limits allowed behavior |
| `validated_by` | Test or fixture | Executable evidence |
| `applies_to` | Code anchor | File, module, symbol, or mode in scope |
| `supersedes` | KnowledgeUnit | Replaces an older semantic identity |

Authors record only forward relations. Reverse indexes are derived.
`depends_on` and `constrained_by` form one required graph and MUST be acyclic.
`supersedes` MUST also be acyclic. Validation and code anchors do not
participate in the required knowledge graph.

Derivation and support belong to provenance. Translation and summarization
belong to projection lineage. A weak `related_to` signal belongs to Librarian
discovery and MUST NOT affect SOT eligibility or invalidation.

Knowledge files pin typed `{ SourceId, SourceRevision }` references. Source
records own their location, original content or external reference, and
revision exactly once. A Source change never follows implicitly: the
KnowledgeUnit must pin the new SourceRevision, producing a new RevisionId that
requires review, approval, and Checkpoint activation. The closed initial Source
kinds are:

| Kind | Meaning |
| --- | --- |
| `decision` | Accepted design decision |
| `code` | Repository path, symbol, and content hash |
| `conversation` | Authorized minimal excerpt or opaque external reference |
| `external` | Document or standard outside the repository |

Code line numbers are hints, not identity. Path and symbol locate a code Source;
its content hash detects drift.

The Pilot stores one typed YAML record per Source below
`methexis/sources/<kind>/`. Directory and filename are organizational hints;
the record's SourceId is identity. The schema is closed and has no catch-all
payload. Conversation records contain either an authorized excerpt or an opaque
reference. External records declare immutable, mutable, or attested freshness,
but Conversation and External records remain ineligible until the corresponding
verifier exists.

`SourceRevision` is `sha256:<lowercase-hex>` over a domain-separated,
length-delimited representation of schema, SourceId, kind, and that kind's
semantic fields. YAML formatting, physical record path, generation time, a code
line hint, and the revision field itself are excluded. Code path, symbol, and
content hash are semantic; a code symbol is a locator rather than a byte-range
extraction boundary.

## SOT-004: Revision, approval, and Checkpoint

`KnowledgeId` is stable semantic identity. `RevisionId` identifies exact
canonical meaning. The Pilot encodes it as `sha256:<lowercase-hex>` over one
unambiguous, length-delimited semantic representation containing:

- schema version, ID, kind, and owner;
- canonical body;
- sorted pinned Source references;
- relation type and sorted target references for every closed relation type.

The loader normalizes CRLF and bare CR to LF before hashing. Other canonical body
bytes remain meaningful. Physical path, YAML key order or formatting, generation
time, and original line-ending representation MUST NOT change the revision.

A revision stays under the same KnowledgeId only while it answers the same
semantic question and existing inbound relations still identify the same
obligation. Clarification, tighter wording, and changed outcomes for that same
obligation are new revisions.

Use a new KnowledgeId plus `supersedes` when the subject or obligation changes
enough that an existing relation would silently acquire a different meaning.
A split creates multiple new IDs that supersede the old unit; a merge creates
one new ID that supersedes multiple old units.

Deterministic validation checks only structural evidence: supersession targets
exist, its graph is acyclic, old and replacement units are not active together,
and no removed ID leaves a required inbound relation unresolved. Librarian MAY
flag overlapping anchors or similar meaning as a possible unexplained
replacement. A human reviewer owns the semantic continuity decision.

Approval binds one exact RevisionId, reviewer OwnerId, review time, and the
profile, compiler identity, and hash of the Korean review projection. Approval
does not apply to a mutable KnowledgeId in general.

The Pilot keeps one generated Korean review Projection per KnowledgeId under
`methexis/review-projections/`. It binds the exact RevisionId, Projection
profile, compiler identity, deterministic request lineage, and exact reviewed
file bytes. Direct edits, revision drift, or lineage drift are structural
failures; the file is regenerated from an explicit request instead.

Each KnowledgeId has at most one current approval record. When the current
revision differs from that record, the unit is Draft. Git history retains old
approval records; the Pilot does not create an unbounded file per historical
revision.

The current approval record lives under `methexis/approvals/`. Identical writes
are idempotent. Replacing different bytes requires the exact prior RevisionId
as a compare-and-swap precondition; there is no force path. A matching record
in a working tree or proposed branch is only approval evidence for review. It
does not produce effective `approved` state until loaded from the configured
trusted integration commit.

A `Checkpoint` pins a consistent map of approved KnowledgeIds to RevisionIds.
A tracked active-Checkpoint record points to exactly one Checkpoint and its
content hash. Activation is a reviewed Git change that adds or updates the
Checkpoint and active record in one commit. It becomes authoritative only when
that commit is reachable from the configured trust anchor. That accepted Git
commit is the atomic authority transition.

An active-record replacement also stores the exact prior trusted active-record
hash used as its compare-and-swap predecessor. The initial activation stores no
predecessor. This lineage is part of the deterministic active-record identity,
so repository review and pre-integration validation can reproduce the
transition instead of trusting that a particular CLI invocation created it.

Any local active pointer is only a reconstructible cache. It MUST be bound to
the Git tree identity and active-Checkpoint hash, replaced crash-safely, and
discarded on mismatch. Concurrent authority changes are serialized by the
repository merge and review workflow rather than a runtime database lock.

The current Checkpoint request names explicit root KnowledgeIds. Selection adds
the complete `depends_on` and `constrained_by` closure; `validated_by` and
`applies_to` do not add KnowledgeUnits. Checkpoint creation reads exact blobs
from one pinned trusted Git commit without checking it out, then publishes an
immutable create-if-absent proposal. Activation proposal is a separate
active-record compare-and-swap operation. It rejects a Checkpoint created from
an older trusted commit and has no fallback or force path. Before proposal, its
canonical bytes MUST be reproduced from the recorded commit. After integration,
that commit MUST be an ancestor of current trusted integration, remain
readable, and reproduce the same Checkpoint while the current approved closure
also matches. A Checkpoint MUST NOT select a replacement together with a unit
it supersedes.

The Checkpoint record retains the historical `source_status: not_evaluated`
input marker because Source freshness is a current derived guard rather than
authored Checkpoint state. Once the Source validation engine is present, Fast
Check derives `active` or `degraded` from the trusted Checkpoint and current
observations. The W0 Draft seed does not create real approvals or activation;
that authority transition belongs to a later directly reviewed Slice. The
current repository has completed that later transition for all five seed units.

## SOT-005: Librarian discovery and location boundary

Final context selection is deterministic. Librarian is an advisory discovery
and catalog component that MAY:

- propose candidate KnowledgeIds and explain each reason;
- map stable semantic IDs to physical locations;
- recommend classification and placement;
- detect duplicates, orphans, and broken references;
- generate reviewable relocation plans.

Librarian MUST NOT approve meaning, mutate canonical authority silently, or
bypass a Checkpoint. Search and LLM output are candidate signals only.

The first agent path accepts a versioned request containing at least one of a
non-empty natural-language query or one or more code-path, symbol, and
KnowledgeId anchors. It returns a deterministic, versioned candidate set.
Each candidate contains a stable KnowledgeId and machine-readable reasons;
Librarian never labels it approved, active, or safe to use. Methexis owns
required-closure expansion and final eligibility filtering.

The initial catalog contains every structurally valid KnowledgeUnit, regardless
of approval or eligibility. Searchable fields are its ID, title, canonical
English body, typed relations, physical location, and an exact-revision valid
Korean review Projection when present. Source content, approvals, and
Checkpoints do not contribute text-ranking signals. Structured code Source
locators MAY satisfy an explicit path or symbol anchor without making Source
content searchable.

Librarian builds that catalog from the current working tree, including valid
untracked Draft records inside the declared corpus directories. It does not
resolve `develop` or grant trust to those files. It captures the sorted relative
paths and exact relevant bytes into one immutable catalog snapshot before
ranking. A concurrent change that prevents a consistent capture returns a
retryable failure and no candidate set.

Initial ranking is deterministic lexical evidence, ordered from exact ID and
anchor matches through phrase and token overlap to one-hop relation signals.
Every contribution remains inspectable in the candidate reasons. Librarian
MUST NOT expand required dependency closure. Semantic or vector retrieval, LLM
ranking, fuzzy matching, and language-specific morphological dependencies
remain evidence-gated extensions rather than Pilot defaults.

An unresolved anchor and a query with no matches are successful observations
and remain explicit in the result. A request with neither query nor anchors is
invalid. An invalid catalog produces a structured failure and no partial
candidate set; silently skipping a damaged record could hide required
knowledge.

The discovery command writes exactly one complete structured success value to
stdout. It writes a structured failure to stderr, leaves stdout empty, and
returns non-zero. Callers MAY pipe or redirect successful JSON to a file; the
Pilot MUST NOT create or own a persistent candidate artifact or cache. The
result identifies the request, catalog snapshot, compiler, ordered candidates,
reasons, unresolved anchors, and truncation. S4 hashes the exact candidate input
it consumes into the ContextBuild lineage.

The Pilot rebuilds one immutable in-memory catalog from the captured
working-tree files for each request. It does not introduce a database,
persistent index, storage trait, or background service before corpus evidence
justifies one. Any later index remains reconstructible and non-authoritative.

The initial Librarian implementation incubates under `yo/tools/librarian`.
Validated capabilities and contract tests later graduate to a standalone
Librarian repository after Surface and SOT operating-procedure dogfooding.
Contract fixtures transfer before implementation, `yo` retains a thin adapter,
reference corpus, contract fixtures, and integration evaluation, and the two
repositories MUST NOT maintain parallel implementations. The destination
repository and reconciliation with any existing Librarian code are decided from
that evidence; the Pilot directory is not copied wholesale.

## SOT-006: Invalidation and validation

Approval and context eligibility are separate derived axes.

| Approval | Trigger |
| --- | --- |
| `draft` | Current revision has no matching trusted approval |
| `approved` | Current revision has an exact matching trusted approval |

| Eligibility | Trigger |
| --- | --- |
| `active` | Included by the active Checkpoint and passes every guard |
| `inactive` | Valid but not included by the active Checkpoint |
| `stale` | Approved Source or evidence freshness no longer matches |
| `suspect` | Exact revision has an unresolved review hold |
| `invalid` | Deterministic integrity failure or explicit invalidation |

Normal context requires both `approved` and `active`. Every other combination is
excluded; suspect and stale content is visible only in a marked diagnostic view,
while invalid content is never emitted as agent context.

Status is not authored on a KnowledgeUnit. When multiple eligibility conditions
apply, precedence is:

```text
invalid > suspect > stale > inactive > active
```

- `invalid`: deterministic schema, graph, integrity, or Checkpoint failure, or
  an explicit human invalidation of the exact revision;
- `suspect`: an explicit review hold on the exact revision for unresolved
  semantic or provenance uncertainty;
- `stale`: a pinned Source, evidence result, retrieval, or attestation no longer
  satisfies its approved freshness input;
- `inactive`: the revision is not selected by the active Checkpoint;
- `active`: the revision is selected and passes every eligibility and freshness
  check.

Durable review holds and invalidations are tracked records. A current
working-tree or host observation MAY only demote eligibility; it cannot grant
approval or activation. Every derived state includes machine-readable evidence
for the winning condition, so precedence and transitions are testable.

A resolution started after a Source change MUST block affected knowledge and
projections and mark affected Checkpoints degraded. A change concurrent with
resolution follows the immutable snapshot and final revalidation rules in
`SOT-007`. Unaffected approved knowledge remains eligible.

Fast editing validation uses two phases. The local phase parses every record and
aggregates schema, field, ID, relation-shape, and body-section diagnostics. The
global phase runs only when every record passes locally, then aggregates
duplicate IDs, missing owners or targets, and graph cycles. Diagnostics have
stable codes and deterministic path/code/location ordering. Any diagnostic
produces no snapshot.

The Pilot exposes that validation as four ordered check classes:

```text
records -> relations -> authority -> artifacts
```

`methexis check` requests all classes. `--only` accepts a repeatable,
comma-separated list, so `--only authority,artifacts` and repeated `--only`
flags are equivalent. A requested class always executes its prerequisites.
The versioned report distinguishes canonical `requested_checks` from
`executed_checks` and records each planned class as `passed`, `failed`, or
`blocked`. A failed prerequisite blocks its remaining dependants rather than
presenting them as checked. Selector names are case-sensitive; surrounding
whitespace is ignored, while unknown names and empty comma segments are usage
errors. A blocked requested class makes the overall report unsuccessful because
the requested validation did not complete.

An agent MAY request bounded successful output with `--summary`. That result
retains the requested and executed classes, their statuses, authority,
affected IDs, and diagnostic count, but omits the complete KnowledgeUnit list
unless one exact ID is selected with `--unit <knowledge-id>`. Unit selection
requires summary output and a requested `authority` or `artifacts` class,
because earlier classes do not derive approval and eligibility. An invalid
combination or unknown unit is a usage failure rather than an empty success.
Output bounding MUST NOT hide validation evidence: every unsuccessful check
returns the complete ordinary report and diagnostics regardless of these
selectors.

Working-tree `methexis check` validates Draft Knowledge, typed Source records,
and any tracked Projection and approval proposals. It MAY report `matching_proposal`,
`stale_proposal`, or missing working-tree evidence, but MUST NOT promote that
evidence to trusted approval. It separately reads the pinned trusted commit and
may report `approved` only for an exact matching approval found there. A trusted
active record becomes `active` only when all selected Source guards pass; a
stable freshness failure yields `degraded` and marks only affected required
closures stale or invalid. Fast editing validation SHOULD be available through
the repository `hk` workflow.

`methexis check --staged-activation` is the repository-hook path for the
otherwise unavoidable interval after revised approvals reach trusted
`develop` and before their replacement Checkpoint is integrated. Without a
staged active-record change it has exactly the ordinary all-class `check`
behavior. With one, it accepts only one new immutable Checkpoint, the active
record, and the complete registered tracked-artifact set in the Git index;
unrelated staged paths fail closed.

The staged path is read-only and prospective, never trusted authority. It
resolves `develop` once, reproduces the proposed Checkpoint from that exact
trusted commit, verifies the active record's exact predecessor hash and
canonical bytes, requires every selected Source to remain fresh, checks staged
artifact provenance, and revalidates Source, proposal-index, and trusted-ref
stability before returning. It pins the exact Git index selected by the commit
invocation, including an explicit `GIT_INDEX_FILE`, and rejects non-regular or
non-stage-zero entries. Success
labels the candidate `prospective` and requires ordinary full `check` after the
exact reviewed transition is integrated. It MUST NOT accept caller-selected
refs, arbitrary future trees, working-tree-only candidate bytes, or a general
hook exception.

This check mechanizes the second half of a two-commit authority transition; it
does not make revised approvals and their Checkpoint one authority commit. The
trusted ref may therefore be intentionally inconsistent between the accepted
approval commit and its exact back-to-back activation commit. During that
bounded interval ordinary `check` and authority-consuming operations continue
to fail or use only the prior still-valid active authority; prospective success
never grants approval, activation, or context eligibility.

The `artifacts` class validates only tracked contract artifacts derived from
trusted authority. In this Pilot it checks the registered context manifests'
Checkpoint ID, hash, and authority-basis commit against the active trusted
Checkpoint. It does not claim byte-for-byte regeneration and does not inspect
or gate rebuildable `.local-exclude/` ContextBuild caches. Generic Rust tests,
linting, and formatting remain Cargo and `hk` responsibilities rather than
Methexis check classes. A repository or isolated fixture with none of the
registered tracked artifact paths has an empty, passing `artifacts` class.
Presence of any registered path enables the closed set, after which every
registered artifact is required. If no active trusted Checkpoint is available,
`authority` may pass as an evaluation while `artifacts` is `blocked`; the
requested validation is incomplete, so the overall report fails and directs
the caller to establish active trusted authority.

Checkpoint activation additionally verifies:

- approval and Source freshness;
- complete required dependency closure;
- exclusion of replaced old knowledge;
- current executable evidence;
- reproducible human-review projection.

Executable evidence is content addressed. Unchanged code, knowledge, command,
and tool inputs reuse prior evidence. Related changes stale only affected
evidence. Context resolution consumes an active Checkpoint and does not rerun
the entire validation suite, but it MUST run the freshness guard defined by
`SOT-007` before using cached eligibility.

## SOT-007: Context resolution and artifacts

Context selection starts from explicit paths, symbols, KnowledgeIds, and
Librarian candidates. The resolver then:

1. resolves one immutable trusted integration commit;
2. loads the tracked active Checkpoint from that commit;
3. captures current local Source bytes and identities into an immutable
   Source snapshot;
4. verifies its cheap freshness guard;
5. filters by active Checkpoint eligibility;
6. expands required and constraining relations;
7. attaches applicable validation evidence;
8. applies priority and token-budget packing;
9. final-revalidates observed mutable Sources;
10. publishes an immutable, traceable `ContextBuild`.

The versioned request MUST contain at least one direct anchor or one Librarian
candidate result reference. A candidate reference is a repository-relative
local path plus the expected SHA-256 of the exact file bytes; the candidate
JSON is not embedded in the request. The resolver captures and verifies those
bytes before parsing and records their hash in ContextBuild lineage. A direct
KnowledgeId, path, or symbol anchor is a required root. An unresolved direct
path or symbol fails explicitly; when it resolves to multiple exact units, all
of them are required roots. Librarian candidates are advisory optional inputs.

Direct anchors resolve only against the KnowledgeSnapshot loaded from the
pinned trusted commit. A KnowledgeId matches its exact semantic ID. A path
matches either the exact canonical repository-relative Knowledge record path or
an exact `applies_to` value; a symbol matches only an exact `applies_to` value.
Code Source locators, Librarian's working-tree catalog, Draft files, and fuzzy
text do not participate. Anchor values use the same typed, trimmed
duplicate-rejection semantics as Librarian requests; the S4 request schema
additionally declares maximum anchor counts and value lengths.

A candidate path must remain beneath the opened repository root. Capture
rejects absolute paths, empty or dot components, `..`, symlinks, non-regular
files, and files over the compiler profile's declared bound. It opens path
components relative to retained directory handles, captures one bounded byte
snapshot, and verifies file identity before and after capture. A concurrent
change is a structured retryable failure with no partial result or automatic
retry.

Methexis validates the candidate wire contract rather than reimplementing
Librarian retrieval. Its independent closed decoder validates every envelope,
identity, compiler, candidate, path, reason, unresolved-anchor, and truncation
field defined by the versioned candidate-set schema. It rejects unknown fields,
duplicate candidates or reasons, collection ordering that the schema declares
canonical, malformed or inconsistent hashes and candidate-set identity, a false
success marker, a candidate score unequal to the sum of its reason scores, and
candidate ordering that is not descending score then ascending KnowledgeId.
Cross-tool golden fixtures pin the complete accepted and rejected wire shapes.
Methexis does not recompute reason signals or fixed score weights, candidate
recall, or whether Librarian found the best result; reason scores determine
advisory order, not authority or eligibility.

The freshness guard runs on every resolution, including a cache hit. It compares
the trusted commit and active-Checkpoint hash, referenced KnowledgeUnit hashes,
approval revisions, and required evidence hashes. For a code Source it resolves
the recorded locator against the current working tree, captures the bytes and
file identity, and hashes that immutable snapshot. A missing locator, dirty
change, or hash mismatch is drift rather than an implicit authority revision.

The code guard hashes exact whole-file bytes in v1; it does not normalize line
endings or extract a symbol range. It walks repository-relative path components
without following symlinks, retains the opened file while capturing bytes,
checks identity before and after capture, and reopens and rehashes immediately
before returning. A stable missing file or hash mismatch is stale, a path escape
or symlink is invalid, and a concurrent identity or byte change returns the
retryable `source_changed_during_validation` failure without a partial result or
automatic retry.

External Sources use one enforceable freshness mode:

- immutable or versioned: verify the pinned identifier and captured hash;
- mutable and retrievable: retrieve current content and compare its hash;
- opaque or unavailable: require a human attestation with a fixed expiry.

Missing retrieval, missing attestation, or expired attestation fails closed.
The Pilot need not implement a generic external connector until its corpus
requires one. The guard does not rerun executable validation.

The resolver compiles only from its captured Source snapshot. Immediately before
publishing a new artifact or returning a cached one, it rechecks every observed
mutable Source identity and hash and compares the current trusted-ref and active
Checkpoint identities with the values captured at operation start. A concurrent
mismatch publishes nothing and returns a structured retryable
`source_changed_during_resolution` or `authority_changed_during_resolution`
failure.
This whole-operation failure also applies when the concurrently changed Source
belongs only to an optional candidate: the resolver cannot claim a consistent
snapshot assembled partly before and partly after that change. It does not
retry automatically.

Selection operates on atomic semantic bundles. A root or candidate and its full
transitive `depends_on` and `constrained_by` closure are either included
together or not included. Shared required units are included and charged once.
A blocked or unaffordable required-root bundle fails the build. A blocked or
unaffordable optional-candidate bundle is omitted as a whole with a structured
manifest reason.

Packing uses deterministic greedy order. Required-root bundles are admitted
first. Optional candidates are then considered in the validated Librarian
order; a bundle is included when its marginal token cost fits, otherwise it is
omitted and later candidates are still considered. The Pilot does not use
score-per-token optimization, knapsack selection, an LLM reranker, or silent
body truncation.

The request names a supported versioned tokenizer profile and a maximum token
budget. The resolver counts the actual tokens of every byte-bearing element in
the final agent payload, including its preamble, stable IDs, headings, bodies,
and emitted relation text. It MUST NOT substitute character or byte estimates.
An unsupported profile is a structured failure. Tokenizer identity and version
are lineage inputs and change the BuildId. The first implementation supports
one profile and pins `o200k_base/v1` to `tiktoken-rs` 0.12.0.

Applicable existing validation evidence is recorded and attached when the
corpus provides it. In the first compiler profile it appears only in the
manifest and consumes no agent-payload tokens. Context resolution does not
execute validation commands, invent missing evidence, or describe a
`validated_by` reference as an executed result. Evidence execution and
collection are a later capability.

Artifact publication is atomic create-if-absent, never replacement. The
publisher builds in a temporary sibling and installs it with a no-clobber
primitive. If the BuildId destination already exists, it verifies the manifest
and every artifact hash and reuses the exact match. Existing-build verification
rejects symlinked build or artifact paths, resolves them relative to retained
directory handles, and retains those handles through verification and result
construction. A mismatch is a determinism or corruption collision: quarantine
the new temporary output, keep the existing artifact unchanged, and fail.
Partial output is never eligible.

A Source, approval, or evidence freshness mismatch degrades the Checkpoint and
fails the affected required dependency closure before context is returned.
Optional affected knowledge is omitted with a structured reason. Local
ContextBuild corruption or a determinism collision fails storage verification
but does not alter Checkpoint eligibility. A resolver MUST NOT fall back to an
older approved revision. A new review, evidence run, and activation restore
eligibility.

Missing, blocked, or unaffordable required knowledge MUST fail the build.
Required bodies MUST NOT be silently truncated. Optional knowledge MAY be
omitted only when the manifest records the omission and reason.

The user operation resolves a context; it does not rebuild one on every
request. Identical content-addressed inputs reuse an existing `BuildId` only
after the freshness guard passes. Relevant knowledge, relation, compiler,
projection, tokenizer, direct-anchor, exact candidate-input bytes, or budget
changes invalidate only affected results. The exact candidate-input hash is a
BuildId identity input; its physical input path is only a locator.

`BuildId` is the domain-separated SHA-256 of a versioned, length-delimited
canonical build plan. The plan contains the active-Checkpoint identity and hash,
its stable authority-basis commit, selected Knowledge revisions and required
relations, deterministic inclusion and omission decisions with their reason
codes, all Source and evidence observations that affected those decisions,
normalized direct anchors, the exact candidate-input hash, compiler and payload
profile, tokenizer profile, and maximum budget. It excludes the current
observation of `develop`, input and output paths, timestamps, result status, and
artifact hashes. Consequently an unrelated trusted-ref advance can reuse the
same build after final authority and freshness verification, while a change to
any relevant semantic input cannot.

The initial resolver request has no model or permission field and the first
profile performs no model- or permission-specific filtering. A future versioned
profile MAY add such inputs only together with their trusted derivation source,
selection semantics, and BuildId participation; a caller string alone cannot
grant content eligibility.

Pilot artifacts remain outside Git history, for example:

```text
.local-exclude/methexis/builds/<BuildId>/
  context.md
  manifest.json
```

`context.md` is the minimal canonical English payload for the agent and is the
only artifact charged to the request token budget. Its versioned compiler
profile fixes the exact preamble, heading grammar, and emitted relation fields.
Units use a deterministic topological order over `depends_on` and
`constrained_by`, with ascending KnowledgeId as the tie-breaker, so required
units precede their consumers. Each unit emits its stable KnowledgeId, exact
canonical English body, and its included required-relation IDs. Golden fixtures
pin the exact payload bytes and token totals. The payload excludes Korean review
Projections, raw Source content, validation evidence, full approval or
Checkpoint records, and retrieval diagnostics.

`manifest.json` records the Checkpoint and its stable authority-basis commit,
exact candidate-input hash, direct anchors, included and omitted revisions and
reasons, blocked inputs, candidate reasons, compiler and profile identity,
tokenizer and budget, BuildId preimage fields, and the `context.md` hash. It
does not contain its own hash. Agent work records the BuildId it consumed.

The fixed BuildId store owns the immutable original in the Pilot. A successful
structured result returns `created` or `reused`, the BuildId, and the paths and
hashes of both artifacts. That per-operation result also records the exact
current trusted commit observed for final verification; it may therefore differ
across safe reuse of the same immutable build. Cache reuse first reproduces the
BuildId plan, verifies current freshness, and verifies the stored manifest and
artifact hashes. Existing different content at the same BuildId is corruption
and MUST NOT be overwritten.

Caller-selected output paths are not part of initial resolution. A later
read/export operation MAY stream a verified artifact to stdout or copy it to a
caller-selected destination without changing the managed original, BuildId,
lineage, or integrity checks.

## SOT-008: Agent-first interface

The primary Pilot consumer is a code agent. Every operation MUST:

- support non-interactive execution;
- expose versioned structured input and output;
- use stable machine-readable error codes within the Pilot version;
- include affected IDs and actionable next steps in failures;
- return paths and hashes instead of streaming large artifacts through stdout;
- derive human-readable output from the same result.

The responsibility surface includes:

| Methexis | Librarian |
| --- | --- |
| Fast check | Candidate discovery |
| Review packet | Catalog integrity check |
| Exact-revision approval record | Relocation plan |
| Checkpoint activation | |
| Context resolution | |

Exact command names and final JSON fields remain provisional. Review never
implies approval. A CLI cannot prove that its caller is human, so approval still
requires explicit human authorization in the repository review flow.

The current agent path uses versioned JSON request files, conventionally under
`.local-exclude/methexis/requests/`. It writes tracked Projection and approval
proposals, and content-addressed review packets under
`.local-exclude/methexis/reviews/`. Requests and local packets are
non-authoritative and MAY be discarded after their paths and hashes are
returned. A future database MAY retain request history for audit or evaluation,
but remains a reconstructible index rather than authority.

The implemented operations are:

```text
author-revision <request.json>   -> derived revision authoring Draft proposals
project-review <request.json>  -> tracked Korean review Projection
build-review <request.json>    -> local packet and manifest
prepare-approval <manifest.json> --reviewer <owner-id> [--replace-current]
                               -> approval-request proposal on stdout only
approve <request.json>         -> tracked exact-revision approval proposal
prepare-checkpoint             -> Checkpoint-request proposal on stdout only
create-checkpoint <request.json> -> immutable trusted-revision Checkpoint proposal
prepare-activation <create-output.json>
                               -> activation-request proposal on stdout only
propose-activation <request.json> -> active-record proposal with compare-and-swap
check [--only <class>[,<class>...]]... [--summary] [--unit <knowledge-id>]
                                -> selected SOT integrity classes and their prerequisites
check --staged-activation       -> ordinary check or one exact staged prospective transition
resolve-context <request.json>  -> immutable ContextBuild locator and hashes
```

`author-revision` collapses the revision-authoring loop into one call: it
accepts new Source content, a new Knowledge body, and/or new Korean review
Markdown, then derives the SourceRevision, the Knowledge source pin and
RevisionId, the replacement Projection, and the review packet, writing the
tracked files as Draft proposals. The unit's single decision Source id and all
other Knowledge metadata are preserved. Approval records MUST NOT be written
by this operation; human approval remains a separate explicit step. Units
that do not pin exactly one `decision` Source fail closed. Writes are
sequential per-file compare-and-swap operations rather than one batch; a
mid-sequence failure names the paths already written, and re-running the same
request converges the remainder.

The `prepare-approval`, `prepare-checkpoint`, and `prepare-activation`
operations remove hand-copied hashes from the review→approval→checkpoint→
activation loop. Each reads values that already exist in the repository — the
review packet manifest, the active Checkpoint roots, or one saved
`create-checkpoint` result — binds them into the exact request wire shape the
next operation consumes, and prints that request JSON on stdout. The authority
boundaries are unchanged: the prepare operations emit proposals only and never
perform the following mutation. `prepare-approval` MUST NOT write
`methexis/approvals/` or record an approval; human authorization remains the
separate explicit `approve` step. Checkpoint and activation preparation MUST
NOT invoke Checkpoint creation or activation. Missing authority inputs — an
unknown reviewer, a `--replace-current` without an existing approval record,
or no active Checkpoint — fail closed with structured diagnostics.

S4 adds context resolution with a versioned request and one structured result.
Success returns only the small artifact locator and integrity record described
by `SOT-007`; the completed context is not streamed implicitly. Failures
distinguish stable required-input failures from retryable concurrent Source or
authority changes. Stable ineligibility or unaffordability of an optional
candidate remains a successful build with an omission record; malformed input
or an integrity failure still fails the operation. Neither direct anchors nor
candidate input may override the trusted commit, active Checkpoint, approval,
or freshness guards.

Every mutation publishes atomically and rejects symlinked output parents.
Publication resolves and retains directory handles before locking or writing;
a concurrent parent rename or symlink swap cannot redirect output outside that
opened repository directory.
Tracked mutations serialize concurrent writers per target. A different
Projection requires its exact prior content hash; a different approval requires
its exact prior RevisionId. Checkpoints are immutable; active-record replacement
requires the exact prior record hash. Failures leave the prior record unchanged
and expose no eligible partial output.

The Pilot MUST be dogfooded during real Codex Surface work. Interface elements
that do not improve safe agent completion SHOULD be removed or reshaped from
evidence rather than preserved for compatibility.

## SOT-009: Rust and process boundaries

After the repository foundation establishes the root Cargo workspace, add
exactly two initial tool crates:

```text
tools/methexis
tools/librarian
```

Each crate contains one library and one thin binary. Internal concerns remain
modules until an independent consumer justifies another crate.

The tools exchange a versioned candidate JSON artifact. Methexis MUST validate
that artifact and MUST NOT depend on Librarian's internal Rust types. Do not add
a shared contract crate in the Pilot.

The first ContextBuild implementation remains inside the existing Methexis
library and thin binary. It MUST NOT add a resolver crate, database, background
service, external connector, HTML view, GUI, or evidence runner. Those remain
separate evidence-gated capabilities.

This split follows lifecycle rather than module count. Both tools incubate in
`yo` and are expected to graduate to standalone repositories. After each
graduation, `yo` retains a thin adapter, reference corpus, contract fixtures,
and integration evaluation rather than a second implementation.

## SOT-010: Evaluation and graduation

The deterministic suite requires:

- identical inputs produce the same BuildId;
- required knowledge recall is 100%;
- exposure for every combination other than `approved` plus `active` is zero;
- changes invalidate only affected results;
- missing required knowledge fails explicitly;
- cached builds are reused.

Run 8–12 representative agent tasks under:

```text
A. existing full developer documentation
B. Librarian search results only
C. Librarian plus SOT ContextBuild
```

The first Surface run uses ten tasks. Six deterministic microtasks cover:

1. overwriting a wide grapheme without orphaning continuation cells;
2. atomically clipping a wide grapheme at the right boundary;
3. emitting the exact diff for a resolved-style change;
4. preserving global row-and-column diff ordering;
5. restoring cursor and terminal state after Inline mode;
6. preserving semantic parity between a completed Surface and its HTML
   projection.

Four real agent tasks cover:

1. adding one bounded Surface operation;
2. rendering one component through `SurfaceView`;
3. extending the typed terminal adapter;
4. diagnosing an injected fixture failure and adding its regression test.

Each A/B/C attempt starts from the same clean repository state and uses the
same task acceptance tests. The evaluator records unavailable environmental
matrix entries separately from deterministic task failures.

Measure task pass rate, tests passed, required-constraint recall, stale
exposure, successful-input tokens, unrelated file changes, and cache reuse.
Condition C must not reduce task pass rate relative to A, must reduce successful
input tokens, and must reduce required-rule omissions relative to B. Do not set
a universal token-reduction percentage before the Pilot establishes a baseline.

Librarian graduation requires the same contract to work for both Surface and
the SOT operating-procedure corpus, no `yo`-specific public types, identity
preservation across relocation, no authority mutation through search,
transferred contract tests, and a passing `yo` Pilot against the final
Librarian.

Methexis repository graduation requires:

- the deterministic suite and A/B/C Pilot evaluation pass in `yo`;
- its public contract contains no TUI- or `yo`-specific types;
- contract, fixture, and failure tests transfer to the standalone repository;
- `yo` passes the same evaluation while consuming standalone Methexis;
- the in-repository implementation shrinks to a thin adapter.

Repository extraction MAY happen after stable `yo` Pilot evidence when the tool
needs an independent release lifecycle. Until a second real product consumer
exists, the standalone project MUST NOT generalize beyond the contract proven
by `yo`.

### Workflow self-hosting

`CONTRIBUTING.md` remains the sole workflow authority during the Pilot. A future
explicit migration MAY make approved workflow KnowledgeUnits canonical and
commit `CONTRIBUTING.md` as their human-readable `DocumentView` Projection.

That migration requires:

- complete rule coverage and semantic-equivalence review;
- explicit human approval of the generated document;
- a generation-drift check in the repository validation path;
- a pinned last-known-good tool and documented recovery procedure;
- one atomic owner transition that removes dual authority.

After migration, contributors change the owning KnowledgeUnits and regenerate
the committed Projection rather than editing it independently. The generated
file remains readable without running Methexis.

## Delivery

The proposed implementation sequence is:

```text
S1 knowledge-foundation
   |
   +-- S2a approval-projection --> S2b checkpoint-proposal --> S2c source-validation --+
   |                                                                                   |
   +-- S3 librarian-discovery ---------------------------------------------------------+--> S4 context-resolution
                                                                                                |
                                                                                          S5 surface-dogfood
```

S2a and S3 may run in parallel after S1. S2b depends on S2a; S2c owns Source
freshness and is the only stage that may open active eligibility. S4 is the
explicit join of S2c and S3. S5 expands the Surface corpus to roughly 20–50
units and runs the 8–12 task evaluation.

Every Slice must provide one end-to-end agent path, versioned structured
output, success and failure fixtures, owner decision references, tests, and
inspectable example output. The Wave MUST NOT redesign the root Cargo
workspace, introduce database authority, or generalize before evaluation.

## Deferred

- Evolution of the initial Markdown/frontmatter schema after corpus authoring
  feedback.
- Exact command spelling and final structured field names.
- Semantic or vector retrieval.
- Database-backed authority.
- Persistent non-authoritative request history and its retention policy.
- Background services and network APIs.
- Cryptographic reviewer identity.
- Generalizing SOT beyond the `yo`-proven contract before a second consumer.
- Librarian product changes before Pilot graduation.
