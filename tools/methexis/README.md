# Methexis Pilot

Run the Pilot from the `yo` repository root. Mutating operations create
reviewable Draft proposals. `check` separately derives exact-revision approval
from the local `develop` ref, but Source freshness is not implemented, so no
Checkpoint can make knowledge active yet.

The versioned contract fixtures under
[`examples/review-contract`](examples/review-contract/) and
[`examples/checkpoint-contract`](examples/checkpoint-contract/) show complete
requests and structured success and failure results. Copy requests to
`.local-exclude/methexis/requests/` before replacing their fixture identity,
revision, hash, reviewer, or review time.

```text
methexis project-review <projection-request.json>
methexis build-review <review-request.json>
methexis approve <approval-request.json>
methexis create-checkpoint <checkpoint-request.json>
methexis propose-activation <activation-request.json>
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
`develop`. Until Source validation lands, `check` reports an integrated active
record as `pending_source_validation` and keeps every unit `inactive`.

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
  mod.rs         CheckpointService and shared wire-contract types
  operations.rs  Checkpoint and activation-proposal orchestration
  git.rs         isolated pinned trusted-ref Git-object snapshot
  validation.rs  approved required-closure selection
  records.rs     deterministic record encoding and validation
  storage.rs     immutable publication and active-record CAS
  evaluation.rs  trusted approval and pending activation derivation

src/publication.rs  directory-handle-relative lock and atomic-write policy

tests/checkpoint_flow/
  contract.rs  executable agent fixtures and authority transition
  lineage.rs   claimed Git provenance reproduction
  replacement.rs active-record compare-and-swap scenarios
  failures.rs  approval, integrity, and trust-movement failures
  support.rs   deterministic Git-backed repository fixture
```

Keep new behavior with its owning module. Add a module only when it gains a
separate invariant or dependency boundary; do not turn individual helper
functions into architectural layers.
