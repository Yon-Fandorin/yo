# Methexis Pilot

Run the Pilot from the `yo` repository root. Its current operations create
reviewable Draft proposals; they do not grant trusted approval or activate a
Checkpoint.

The versioned contract fixtures under
[`examples/review-contract`](examples/review-contract/) show complete requests
and structured success and failure results. Copy requests to
`.local-exclude/methexis/requests/` before replacing their fixture identity,
revision, hash, reviewer, or review time.

```text
methexis project-review <projection-request.json>
methexis build-review <review-request.json>
methexis approve <approval-request.json>
methexis check
```

`project-review` writes a generated file under
`methexis/review-projections/`. `build-review` returns the path and hash of a
local packet under `.local-exclude/methexis/reviews/`. After a human explicitly
accepts that exact packet, `approve` writes a tracked proposal under
`methexis/approvals/`.

Every operation prints one JSON value. Success uses stdout and exit code `0`;
failure uses stderr and exit code `2`. Treat returned paths and hashes as the
handoff contract instead of scraping prose.

## Code map

The review workflow stays one crate-level concern with a small facade:

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
```

Keep new behavior with its owning module. Add a module only when it gains a
separate invariant or dependency boundary; do not turn individual helper
functions into architectural layers.
