# Review and integration

This file is the repository workflow authority for required review lenses,
verdict evidence, approval, integration, and cleanup. It is routed directly
from [`AGENTS.md`](../AGENTS.md).

Each Slice must include its implementation or docs, discriminating validation, public-contract updates, and known limits.

## Required lenses

Every Slice receives worker self-check and its required review lenses. The
slice planner proposes its risk and required lenses in the Slice Contract;
workers cannot lower them. Escalate risk when implementation reveals
public-contract, failure-behavior, or shared-ownership impact.

Fresh-context contract review is required for public contracts, terminal lifecycle, concurrency, failure behavior, workflow, and SOT changes. Slice integration review is required for Wave Slices and changes that consume shared interfaces or sibling results. A simple independent docs or configuration Slice may omit an additional lens only with a recorded rationale.

Every Slice that changes implementation code, executable validation code, or
Developer Docs theme source receives a code-quality review. This lens checks
responsibility and module boundaries, duplication, naming, unnecessary
abstraction, complexity, diagnostics, cleanup, and test maintainability. The
same fresh-context reviewer may perform both contract and code-quality review,
but must inspect and record the lenses separately.

## Review protocol

Review packet construction and preparation are owned by
[Review packets](review-packets.md). External authorization, delivery,
continuation, and reviewer execution are owned by
[Review delivery](review-delivery.md). Read the applicable owner before
preparing or accepting review evidence.

## Evidence and disposition

The accepted commit records completed lenses as evidence, not as a substitute
for performing them. Put every trailer in one contiguous block at the very end
of the commit message, with no blank lines between trailers and no body text
after them. For example, when all three review lenses apply, the final block is:

```text
Slice-Review: fresh-context - completed - codex/7f3a91 - clear
Slice-Review: code-quality - completed - kimi/2b8c44 - resolved
Slice-Review: integration - completed - human/minseo - clear
```

Use `clear` when the completed review found no actionable findings. Use
`resolved` only after findings were addressed and the same lens re-reviewed the
final diff with no remaining actionable finding. Reviewer IDs are compact
tokens such as `kimi/session-id`, `codex/session-id`, or `human/name`; put
operational detail in the Slice status rather than free text in the trailer.

Accepted commits prepared from review-coverage cutover
`edf376fd33dc10e8fa3e02ca0e4543025249838a` or its descendants also record one
exact ledger entry for every completed lens:

```text
Review-Coverage: fresh-context - exact - model-high/codex/gpt-5.6-sol/session-id - sha256:<canonical-diff>
Review-Coverage: code-quality - exact - model/codex/gpt-5.6-luna/session-id - sha256:<canonical-diff>
Review-Coverage: fresh-context - exact - delegated-high/codex/session-id - sha256:<canonical-diff>
```

`model-high/<provider>/<model>/<session>` records an independently selected
high-capability model and is required for model-performed fresh-context and
integration review. Mechanical code-quality review may use
`model/<provider>/<model>/<session>`. The provider and session must reproduce
the compact `<provider>/<session>` identity in the matching `Slice-Review`.
This records the actual route without hard-coding one vendor as policy.
`delegated-high/<host>/<session>` and `delegated/<host>/<session>` record the
same lens classes for a host-owned review whose downstream Provider and model
are not visible to Yo. They reproduce `<host>/<session>` in `Slice-Review` and
must not invent downstream coordinates.

A person may perform any lens instead:

```text
Slice-Review: fresh-context - completed - human/yon - clear
Review-Coverage: fresh-context - exact - human/yon - sha256:<canonical-diff>
```

Human coverage means that named person inspected the exact patch for that
lens and explicitly returned the recorded verdict. A general `go`, standing
authorization, integration approval, or review of an earlier candidate is not
human review evidence. Human and model review use the same exact-diff gate;
neither is a bypass for the other validation requirements.

The ledger hash is SHA-256 of the canonical binary, full-index, no-renames
base-to-candidate diff carried as `git_diff` by the immutable review packet.
At accepted squash time, commit preflight recomputes the same bytes from the
integration `HEAD` and staged index. Slice close recomputes them from the
accepted commit and its first parent. A changed integration surface therefore
requires review again instead of inheriting an earlier verdict. Commits at or
before the cutover retain their historical evidence and are not backfilled.

After that cutover, accepted integration branches do not use
`git commit --amend`, `-c`, or `-C`: those operations can combine an earlier
accepted surface with only an incremental staged diff. Working `slice/`, `task/`, and
`spike/` branches may still rewrite their disposable candidate history. When
an accepted change needs correction, prepare the complete message in a file
and create a new commit with `cargo xtask slice commit <message>` so both
preflight and Slice close observe the same first-parent surface. The command
invokes a non-amend Git commit through the ordinary editor boundary; the
`prepare-commit-msg` hook rejects ambiguous `-m`, `-F`, `-t`, `-c`, `-C`, and
`--amend` operations before the message is edited. A person may instead use
plain `git commit` without a configured message template, read the complete
message in the editor, and accept it there.

When no additional lens applies, record `Slice-Review: none - <reason>`.
`cargo xtask check slice-review-impact` reads the final trailer block and fails
closed when the disposition is missing or malformed. It requires
fresh-context review for product/shared/tool code, build/Cargo metadata, workflow, and
semantic SOT; code-quality for executable `crates/`, `shared/`, and `tools/` source plus
Developer Docs theme source; and integration review on Wave branches. This is
a minimum path-based safety net, so the planner adds lenses required by semantic
impact. `none` cannot accompany completed lenses, and unfinished or unresolved
reviews never satisfy a lens. Existing accepted commits keep their historical
trailers; new commits and amends use this grammar.

Classify a Slice as **human-attention** when it introduces or changes a product
decision, public contract, failure semantics, dependency choice, permissions,
destructive or external effect, workflow authority, or semantic SOT authority;
when required review has an unresolved finding; when validation is not green;
or when material uncertainty remains. Uncertain classification is
human-attention. It requires explicit human approval for that exact Slice.

A Slice is **routine** only when it implements an already approved exact
contract or performs mechanical repository follow-through, introduces none of
the human-attention conditions, has all declared dependencies accepted and
integrated, passes all required validation, and has no unresolved finding from
any review. Under a standing human authorization, the slice planner may
auto-integrate a routine Slice and MUST record the classification rationale
plus the authorization's human origin and scope in the integration report or
accepted commit. Generated projections, fixtures, approval records, and
immutable Checkpoints are routine only when they preserve an already reviewed
exact revision and add no semantic choice. Selecting roots or changing the
active Checkpoint is always human-attention. Explicit human approval that
already includes the exact activation transition satisfies that Slice's
disposition; do not request a second merge approval. If the standing
authorization is absent or revoked, routine Slices also require explicit human
approval.

## Integration and cleanup

Run the declared baseline on the reviewed candidate. After squash, reuse its
result only under the exact conditions in the Developer Docs
[Slice-close baseline](../docs/src/validation/README.md#slice-close-baseline);
otherwise rerun the affected gates and lenses. Evidence reuse never replaces
candidate validation or fresh-context review.

After the Slice's review disposition is satisfied, squash a direct Slice into
`develop`:

```bash
git switch develop
git merge --squash slice/direct/<slice>
git commit
```

Squash an accepted Wave Slice into its Wave:

```bash
git switch wave/<wave>
git merge --squash slice/<wave>/<slice>
git commit
```

The resulting commit is the durable review unit; Task commits are not preserved. A Wave keeps one commit per accepted Slice and must remain green after each one. Handle integration fixes as separately reviewed Slices.

After the accepted commit exists, close its local Slice in two explicit steps
from the integration worktree:

Prefer deriving
`.local-exclude/coordination/<slice>/close-metrics.json` from the same ready
gate after the accepted commit exists:

```bash
cargo xtask slice close prepare \
  .local-exclude/coordination/<slice>/close-prepare.json
```

The experimental `yo.slice-close-prepare-request/v1alpha1` input names the
Slice and gate request, then records only data the gate cannot know: execution
lanes, review rounds and finding dispositions, review-packet totals, the
elapsed bottleneck, and an optional one-to-one command mapping for each known
unverified environment. Relative `gate_request_path` values resolve from the
shared workspace root. The command finds the already accepted matching patch,
re-evaluates the ready gate in the registered Slice worktree, derives the exact
candidate and accepted commit, and copies the gate-verified validation names,
argv, status, and reuse disposition. It atomically publishes only the standard
metrics file and never commits, plans, or cleans up.

```json
{
  "schema": "yo.slice-close-prepare-request/v1alpha1",
  "slice": "example",
  "gate_request_path": ".local-exclude/coordination/example/gate.json",
  "execution_lanes": [{
    "lane": "integration",
    "mode": "serial",
    "operation_count": 1,
    "max_concurrency": 1
  }],
  "review": {
    "rounds": 1,
    "findings": {
      "reported": 0,
      "resolved": 0,
      "not_reproduced": 0,
      "accepted_limits": 0,
      "remaining": 0
    }
  },
  "review_packets": {
    "publication_count": 0,
    "total_managed_tokens": 0,
    "largest_sections": [],
    "reused_inputs": []
  },
  "unverified_validation": [],
  "elapsed_bottleneck": {
    "name": "independent review",
    "elapsed_milliseconds": 45000
  }
}
```

Writing the complete standard file directly remains supported. Its shape is:

```json
{
  "schema": "yo.slice-close-metrics/v1",
  "slice": "example",
  "slice_candidate": "<full-Slice-HEAD>",
  "accepted_commit": "<full-accepted-commit>",
  "execution_lanes": [
    {
      "lane": "cargo_validation",
      "mode": "serial",
      "operation_count": 3,
      "max_concurrency": 1
    },
    {
      "lane": "integration",
      "mode": "serial",
      "operation_count": 1,
      "max_concurrency": 1
    }
  ],
  "review": {
    "rounds": 2,
    "findings": {
      "reported": 1,
      "resolved": 1,
      "not_reproduced": 0,
      "accepted_limits": 0,
      "remaining": 0
    }
  },
  "review_packets": {
    "publication_count": 2,
    "total_managed_tokens": 32000,
    "largest_sections": [
      {
        "kind": "git_diff",
        "name": "base-to-candidate",
        "rendered_bytes": 24000,
        "rendered_tokens": 6000
      }
    ],
    "reused_inputs": ["context-build/sha256:<content-id>"]
  },
  "validation": [
    {
      "name": "workspace-tests",
      "argv": ["cargo", "test", "--workspace", "--all-targets"],
      "runs": 1,
      "status": "passed",
      "reused": false
    }
  ],
  "elapsed_bottleneck": {
    "name": "independent review",
    "elapsed_milliseconds": 45000
  },
  "known_unverified_environments": []
}
```

Lane names are `discovery`, `editing`, `review`, `cargo_validation`, and
`integration`; modes are `parallel` and `serial`. Record only lanes that ran,
but always record integration. Cargo-heavy validation and shared integration
must be serial with `max_concurrency: 1`; other lanes report their actual mode.
Validation status is `passed` or `unverified`; an unverified item has zero runs,
cannot be reused, and names its missing environment in
`known_unverified_environments`. Human review may legitimately have zero
published packets. Packet measurements are compact close diagnostics, not a
replacement for their immutable manifests.

The close planner requires this standard file, checks internally reconcilable
counts and exact candidate/accepted-commit identity, and binds its path and
hash into the plan. Apply revalidates the same bytes before cleanup. These
checks prevent stale, contradictory, or silently edited records; they do not
prove that the reported operations ran. Root self-check and completed review
still own factual accuracy. Delete the local metrics with other completed Slice
coordination after promoting any aggregate lesson to its proper owner.

```bash
cargo xtask slice close plan <slice> /tmp/<slice>-close.json
cargo xtask slice close apply /tmp/<slice>-close.json
```

Review the directly published, hash-addressed plan; do not copy, reserialize, or
edit it. Store it outside the worktree and Slice coordination directory it
closes. `plan` requires clean
integration and Slice worktrees, the bound contract, accepted review evidence,
and exact Slice/accepted-commit patch identity. It fixes refs, paths, binding,
effects, and the coordination entries covered by cleanup. Generate a fresh plan
if integration advances. Newly generated plans use the experimental
`yo.slice-close-plan/v1alpha1`. A v2 or v3 plan remains resumable under its original
identity and safety checks only when its accepted commit predates the tracked
close-metrics cutover marker; v4 keeps its metrics-bound retained-entry
semantics. Commits containing that marker require v4 or newer even if a caller
rewrites and rehashes a legacy-shaped plan.

`apply` revalidates that state and the v1alpha1 cleanup-entry list, then removes the
registered worktree and binding, the complete standard
`.local-exclude/coordination/<slice>` directory, and the expected local Slice
branch. Slice coordination is temporary: promote stable decisions first and
move genuinely unresolved work to the local backlog before planning close.
The plan binds at most 256 recursively enumerated cleanup paths and apply
rejects any added or removed entry before deletion.
Nonstandard contract paths remain preserved under their own owner. The entire
worktree, including ignored output, is removed, so move retained material
first. A repository lock, final
worktree check, and compare-and-swap ref transaction protect cooperating
applies; raw Git does not use the lock. A stale plan fails closed, while an
interrupted apply can resume the planned contract or branch deletion.

## Wave promotion

Never rebase accepted Slice commits. Parallel sibling Waves may finish in any order, but promotion into `develop` is serialized. Before promotion, a Wave based on an older `develop` merges the latest `develop` into the Wave:

```bash
git switch wave/<wave>
git merge develop
```

If that merge exposes a contract conflict, stop and reconcile the owning SOT before code. If it exposes only a mechanical conflict, use a neutral fresh-context integrator and review the resolution. Rerun Wave integration validation against the combined history.

When the Wave exit gate passes, promote it to `develop` with accepted Slice commits intact:

```bash
git switch develop
git merge --ff-only wave/<wave>
```

Squash an approved `hotfix/*` into `main` and carry it into `develop`. Discard `spike/*`; never merge it.
