# Methexis Pilot

Run the Pilot from the `yo` repository root. Mutating operations create
reviewable Draft proposals. `check` separately derives exact-revision approval
from the local `develop` ref and evaluates pinned Source freshness before an
integrated Checkpoint can make knowledge active.

The versioned contract fixtures under
[`examples/review-contract`](examples/review-contract/),
[`examples/author-contract`](examples/author-contract/), and
[`examples/checkpoint-contract`](examples/checkpoint-contract/) show complete
requests and structured success and failure results. The
[`examples/prepare-approval-contract`](examples/prepare-approval-contract/),
[`examples/prepare-checkpoint-contract`](examples/prepare-checkpoint-contract/),
and
[`examples/prepare-activation-contract`](examples/prepare-activation-contract/)
directories pin the request-preparation handoffs between those commands. The
[`examples/context-contract`](examples/context-contract/) directory additionally
pins exact agent payload and manifest bytes. Copy requests to
`.local-exclude/methexis/requests/` before replacing their fixture identity,
revision, hash, reviewer, or review time.

```text
methexis capabilities
methexis author-revision <author-request.json>
methexis project-review <projection-request.json>
methexis build-review <review-request.json>
methexis prepare-approval <manifest.json> --reviewer <owner-id> [--replace-current]
methexis approve <approval-request.json>
methexis prepare-checkpoint
methexis create-checkpoint <checkpoint-request.json>
methexis prepare-activation <create-output.json>
methexis propose-activation <activation-request.json>
methexis refresh-context-manifests <activation-request.json>
methexis resolve-context <context-request.json>
methexis check
methexis check --only authority,artifacts
methexis check --summary
methexis check --summary --unit tui.surface.grapheme-cells
methexis check --staged-activation
```

`check` is read-only. Without selectors it runs all ordered classes:
`records`, `relations`, `authority`, and `artifacts`. `--only` may be repeated
or contain comma-separated names; requested downstream classes automatically
run their prerequisites. The JSON result separates `requested_checks` from
`executed_checks` and marks every planned class `passed`, `failed`, or
`blocked`.

`--summary` bounds a successful result to the check statuses, authority,
affected IDs, diagnostic count, and explicitly selected unit. Add `--unit
<knowledge-id>` when one exact revision is needed. Unit selection requires
`--summary` plus an `authority` or `artifacts` request because earlier check
classes do not derive approval and eligibility; invalid combinations and
unknown IDs fail instead of returning an empty success. Without `--unit`,
summary output omits the full unit list. Validation failures always retain the
complete ordinary report and diagnostics on stderr, even when bounded output
was requested.

`methexis/negative-records.yaml` is a required, tracked authority input. The
canonical evaluated-empty form is:

```yaml
schema: methexis.negative-records/v1alpha1
records: []
```

Its absence, unreadability, symlink substitution, malformed bytes, unknown
fields, duplicate entries, or noncanonical ordering fails closed; an empty
file is not equivalent to an evaluated-empty manifest. Each record binds one
exact `knowledge_id` and SHA-256 `revision` to `suspect` or `invalid`, a
tracked `recorded_by` OwnerId, and structured `evidence.code` plus
`evidence.reference`. Matching records demote eligibility in the order
`invalid > suspect > stale > active` and propagate only through required
knowledge relations. Trusted and working records are unioned during runtime,
so deleting a trusted hold only in the working tree cannot clear it, while a
new working record can immediately demote use. Resolve a record through an
ordinary reviewed removal from the tracked manifest; Git history retains the
prior decision.

`check --staged-activation` is the fail-closed repository-hook entry point.
When the index does not contain an active-record change it runs the ordinary
all-class check. When it does, the index must contain exactly one new immutable
Checkpoint, the active record, and both registered context manifests. The
command reproduces that candidate from current trusted `develop`, verifies the
active record's persisted compare-and-swap predecessor, validates Source
freshness and staged artifact provenance, pins the exact commit-selected Git
index (including `GIT_INDEX_FILE`), rechecks it before return, and reports
`authority: prospective`. A degraded candidate fails. The command neither
changes authority nor permits unrelated staged files; ordinary `check` remains
required after integration.

`artifacts` covers the tracked context-contract manifests registered by the
Pilot. It verifies their Checkpoint provenance against active trusted
authority. It neither regenerates the full bytes nor checks rebuildable
`.local-exclude/` caches. Workspace tests and linting remain Cargo and `hk`
responsibilities. If any registered tracked manifest is present, the complete
registered set is required. Without active trusted authority the class is
`blocked` and the requested check is unsuccessful.

`capabilities` reports complete supported workflow profiles. Callers use
membership of `semantic-first-ko-on-demand/v1` to select the v1alpha2 authoring
request; unknown or absent membership keeps the v1alpha1 compatibility path.

`author-revision` v1alpha2 accepts new Source content and/or a canonical
English Knowledge body and writes only those tracked Draft proposals. It
rejects Korean Markdown and does not create, copy, or replace a Projection or
review packet. After repository semantic review clears, `project-review`
receives the exact revision and Korean Markdown explicitly. The v1alpha1
compatibility request remains supported: it accepts any of Source, Knowledge, and
Korean review Markdown and derives the matching Projection and review packet
in the same call. Neither path touches approval records. Both fail closed for
units that do not pin exactly one `decision` Source and use sequential
per-file compare-and-swap publication.

`project-review` writes a generated file under
`methexis/review-projections/`. `build-review` returns the path and hash of a
local packet under `.local-exclude/methexis/reviews/`. After a human explicitly
accepts that exact packet, `approve` writes a tracked proposal under
`methexis/approvals/`.

The three `prepare-*` commands remove hand-copied hashes between these steps.
Each binds values that already exist in the repository into the exact request
wire shape the next command consumes, and prints that proposal request JSON on
stdout; none of them performs the next step's mutation. `prepare-approval`
reads a review packet manifest and emits the approval request with the current
UTC `reviewed_at`; it validates `--reviewer` against the tracked Owner
foundation, binds the current record's RevisionId as `replace_revision` only
with `--replace-current`, and never writes `methexis/approvals/` — human
approval remains the separate explicit `approve` step. `prepare-checkpoint`
reads the working-tree active record and emits a Checkpoint request carrying
the currently active roots, failing closed when no active Checkpoint exists.
`prepare-activation` reads one saved `create-checkpoint` result and emits the
activation request, binding the working-tree active record's content hash as
the compare-and-swap `replace_active_hash` when one exists, and failing closed
for input that is not a successful `create_checkpoint` result.

`create-checkpoint` resolves `refs/heads/develop` once through an isolated
system Git process, disables replacement refs, reads exact Git objects without
switching branches, and writes an immutable Checkpoint proposal containing the
requested roots plus their `depends_on` and `constrained_by` closure.
`propose-activation` reproduces that exact Checkpoint from its claimed commit,
then writes the active-record proposal with compare-and-swap. Replacements
persist the exact prior active-record hash as deterministic lineage, allowing a
later staged check to reproduce the transition without trusting invocation
history.
Both success results include `checkpoint_delta`, computed against the active
Checkpoint in that same pinned snapshot. It identifies the baseline (or
explicitly reports its absence), identifies the immutable candidate artifact,
lists only sorted root-presence and KnowledgeId/RevisionId changes, and reports
candidate and unchanged unit counts. The candidate Checkpoint remains the owner
of the complete closure and selection reasons; successful output does not
repeat unchanged units. Failure output retains its complete diagnostic
`affected_ids`.
Both files become authority only after repository review integrates them into
`develop`. `check` reports a fully fresh integrated active record as `active`;
stable Source drift yields `degraded`, while a concurrent Source change returns
a retryable failure without partial state.

`refresh-context-manifests` prepares the closed registered context-manifest
set for that exact activation proposal. It reuses the activation request,
pins current trusted `develop`, verifies the prospective Checkpoint and the
working active record's canonical compare-and-swap lineage, then recompiles
each manifest with the ContextBuild compiler. The command writes manifests
only when every compiled `context.md` still matches its tracked golden bytes;
a payload change fails and remains a separately reviewed semantic change.
All outputs are computed before publication. A durable transaction journal
then makes the registered set one operation: late failure rolls every changed
manifest back, and the next invocation recovers an interrupted prepared or
committed batch before doing new work. Ambiguous journal or target bytes fail
closed rather than overwriting either version. Methexis writers serialize on
process-lifetime kernel locks, and Methexis readers reject a live journal, so
cooperating commands never accept an intermediate batch. Raw filesystem tools
that ignore those locks are outside this guarantee: they may observe or create
intermediate bytes, and a cooperating refresh may overwrite them or later
Methexis validation may diagnose them as a conflict; raw writes receive no CAS
guarantee.
Run it before staging the four-file transition checked by
`check --staged-activation`.

`resolve-context` accepts required direct anchors and/or a hash-pinned Librarian
candidate result. It verifies trusted approval and freshness, packs complete
required-relation bundles with `o200k_base/v1`, and returns paths and hashes for
an immutable `context.md` and `manifest.json` under
`.local-exclude/methexis/builds/`. Identical relevant inputs reuse the same
BuildId after final Source and authority revalidation.

Every operation prints one JSON value. Success uses stdout and exit code `0`;
failure uses stderr and exit code `2`. Treat returned paths and hashes as the
handoff contract instead of scraping prose.

## Code map

Review and Checkpoint workflows remain crate-internal concerns with small
service facades:

```text
src/author/
  mod.rs          AuthorService facade and exact-version dispatch
  shared.rs       shared Source and Knowledge derivation and publication
  records.rs      deterministic Source record and Knowledge unit encoding
  v1alpha1/mod.rs v1alpha1 request, response, Projection, and packet flow
  v1alpha2/mod.rs v1alpha2 request, response, and semantic-only flow

src/review/
  mod.rs         ReviewService and shared wire-contract types
  operations.rs  Projection, packet, and approval orchestration
  prepare.rs     approval-request preparation from a packet manifest
  records.rs     deterministic record encoding and validation
  storage.rs     atomic publication, CAS, and path safety
  validation.rs  repository-wide proposal state and diagnostics

tests/author_flow/
  contract.rs    agent fixtures, the happy path, and packet equivalence
  failures.rs    input, unit-shape, and validation failures
  support.rs     isolated repository fixture

tests/prepare_flow/
  contract.rs    agent fixtures for the three prepare commands
  chain.rs       the full review-to-activation loop through prepare output
  failures.rs    reviewer, replacement, active-state, and input failures
  support.rs     Git-backed isolated repository fixture

tests/review_flow/
  contract.rs     agent fixtures and the complete happy path
  replacement.rs CAS replacement scenarios
  failures.rs     input, evidence, drift, and filesystem failures
  support.rs      isolated repository fixture

src/checkpoint/
  mod.rs         CheckpointService facade and shared wire-contract types
  candidate.rs   shared prospective authority and final freshness guard
  context.rs     Context authority capture and final revalidation guard
  context_tests.rs final concurrent authority-change regression
  operations.rs  Checkpoint and activation-proposal orchestration
  prepare.rs     Checkpoint and activation request preparation
  prospective.rs exact staged activation and artifact validation
  refresh.rs     prospective authority preparation for manifest refresh
  git.rs         isolated pinned trusted-ref Git-object snapshot
  git/proposal.rs read-only captured-index and parent snapshot for hook validation
  git/tests.rs   captured proposal mutation regression
  validation.rs  approved required-closure selection
  records.rs     deterministic record encoding and validation
  storage.rs     immutable publication and active-record CAS
  evaluation.rs  trusted approval, activation, and freshness derivation

src/source/
  mod.rs          Source facade and eligibility result types
  negative.rs     exact-revision negative records and demotion evidence
  records.rs      typed YAML record loading
  revision.rs     deterministic SourceRevision identity
  validation.rs   closed schema and semantic-field validation
  freshness.rs    selected-unit guards and required-state propagation
  working_tree.rs exact-byte, symlink-safe code capture and revalidation
  tests.rs        revision, schema, drift, race, and propagation scenarios

src/publication.rs  directory-handle-relative lock and atomic-write policy

src/check.rs        check wire types plus record and relation validation
src/check/
  runner.rs         prerequisite planning, execution, and report composition
  artifacts.rs      tracked authority-derived manifest provenance
  artifacts/tests.rs matching and stale-provenance regressions

src/context/
  mod.rs         ContextService facade
  operations.rs  request-to-publication orchestration
  registry.rs    typed owner of registered request, payload, and manifest triples
  refresh.rs     captured compilation and recoverable batch publication
  wire.rs        independent versioned request, result, and candidate structs
  payload.rs     canonical Markdown, actual token count, BuildId, and manifest
  storage.rs     verified reuse, atomic publication, and collision quarantine
  hash.rs        domain-separated identities
  candidate/
    mod.rs        candidate boundary facade
    capture.rs    bounded symlink-safe exact-byte capture and final guard
    validation.rs closed independent Librarian wire validation
    tests.rs      concurrent candidate mutation regression
  selection/
    mod.rs        required-first and optional greedy packing
    anchors.rs    pinned trusted-snapshot exact anchor mapping
    graph.rs      atomic required closure and dependency-first order
    state.rs      eligibility observations and omission reasons

tests/checkpoint_flow/
  contract.rs  executable agent fixtures and authority transition
  lineage.rs   claimed Git provenance reproduction
  prospective.rs exact staged transition and scope rejection
  replacement.rs active-record compare-and-swap scenarios
  failures.rs  approval, integrity, and trust-movement failures
  support.rs   deterministic Git-backed repository fixture

tests/context_flow/
  contract.rs deterministic closure, packing, reuse, and BuildId behavior
  failures.rs stale, budget, corruption, and symlink failures
  support.rs   candidate wire builder and request helpers

tests/context_golden.rs exact tracked payload, manifest, and failure fixtures
```

Keep new behavior with its owning module. Add a module only when it gains a
separate invariant or dependency boundary; do not turn individual helper
functions into architectural layers.
