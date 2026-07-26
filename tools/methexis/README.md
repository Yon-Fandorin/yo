# Methexis Pilot

Run the Pilot from the `yo` repository root. Mutating operations create
reviewable Draft proposals. `check` separately derives exact-revision approval
from the local `develop` ref and evaluates pinned Source freshness before an
integrated Checkpoint can make knowledge active.

The versioned contract fixtures under
[`examples/review-contract`](examples/review-contract/) and
[`examples/checkpoint-contract`](examples/checkpoint-contract/) show complete
requests and structured success and failure results. The
[`examples/context-contract`](examples/context-contract/) directory additionally
pins exact agent payload and manifest bytes. Copy requests to
`.local-exclude/methexis/requests/` before replacing their fixture identity,
revision, hash, reviewer, or review time.

```text
methexis project-review <projection-request.json>
methexis build-review <review-request.json>
methexis approve <approval-request.json>
methexis create-checkpoint <checkpoint-request.json>
methexis propose-activation <activation-request.json>
methexis resolve-context <context-request.json>
methexis check
```

`project-review` writes a generated file under
`methexis/review-projections/`. `build-review` returns the path and hash of a
local packet under `.local-exclude/methexis/reviews/`. After a human explicitly
accepts that exact packet, `approve` writes a tracked proposal under
`methexis/approvals/`.

`create-checkpoint` resolves `refs/heads/develop` once through an isolated
system Git process, disables replacement refs, reads exact Git objects without
switching branches, and writes an immutable Checkpoint proposal containing the
requested roots plus their `depends_on` and `constrained_by` closure.
`propose-activation` reproduces that exact Checkpoint from its claimed commit,
then writes the active-record proposal with compare-and-swap.
Both files become authority only after repository review integrates them into
`develop`. `check` reports a fully fresh integrated active record as `active`;
stable Source drift yields `degraded`, while a concurrent Source change returns
a retryable failure without partial state.

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
src/review/
  mod.rs         ReviewService and shared wire-contract types
  operations.rs  Projection, packet, and approval orchestration
  records.rs     deterministic record encoding and validation
  storage.rs     atomic publication, CAS, and path safety
  validation.rs  repository-wide proposal state and diagnostics

tests/review_flow/
  contract.rs     agent fixtures and the complete happy path
  replacement.rs CAS replacement scenarios
  failures.rs     input, evidence, drift, and filesystem failures
  support.rs      isolated repository fixture

src/checkpoint/
  mod.rs         CheckpointService facade and shared wire-contract types
  context.rs     Context authority capture and final revalidation guard
  context_tests.rs final concurrent authority-change regression
  operations.rs  Checkpoint and activation-proposal orchestration
  git.rs         isolated pinned trusted-ref Git-object snapshot
  validation.rs  approved required-closure selection
  records.rs     deterministic record encoding and validation
  storage.rs     immutable publication and active-record CAS
  evaluation.rs  trusted approval, activation, and freshness derivation

src/source/
  mod.rs          Source facade and eligibility result types
  records.rs      typed YAML record loading
  revision.rs     deterministic SourceRevision identity
  validation.rs   closed schema and semantic-field validation
  freshness.rs    selected-unit guards and required-state propagation
  working_tree.rs exact-byte, symlink-safe code capture and revalidation
  tests.rs        revision, schema, drift, race, and propagation scenarios

src/publication.rs  directory-handle-relative lock and atomic-write policy

src/context/
  mod.rs         ContextService facade
  operations.rs  request-to-publication orchestration
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
