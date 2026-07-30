# Contributing to yo

Make tracked changes as reviewable Slices. Add a Wave only when multiple Slices share an executable milestone, dependency graph, parallel join, or common exit gate. Add Task branches only when workers need isolation. This file is the repository workflow authority.

## Work units

- **Wave**: an optional executable milestone owned by a human and a wave coordinator. Git boundary: `wave/<wave>`.
- **Slice**: one independently reviewable outcome owned by a human and a slice planner. Git boundary: `slice/direct/<slice>` or `slice/<wave>/<slice>`.
- **Task**: one narrow implementation, test, docs, research, capture, or review job owned by a worker. Optional Git boundary: `task/direct/<slice>/<task>` or `task/<wave>/<slice>/<task>`.

The planner is a role, not another work unit or branch layer. If a Slice exceeds one planner's context or a human-reviewable diff, split it into sibling Slices. Do not create recursive planning layers.

Use a direct Slice when the outcome can be independently accepted into `develop`. Use a Wave when its Slices jointly produce one milestone or require coordination. Slice count alone does not require a Wave.

For Waves, use stage IDs such as `w0`, `w1-1`, and `w1-2`. A numeric suffix denotes parallel siblings at the same dependency stage, not another hierarchy. Never renumber or reuse an ID after work starts.

Name branches for outcomes, not workers:

```text
slice/direct/wave-workflow
task/direct/wave-workflow/branch-validation

wave/w1-1-structured-core
slice/w1-1-structured-core/surface
task/w1-1-structured-core/surface/wide-grapheme
```

Use `hotfix/*` only for a released `main` emergency. Use `spike/*` for disposable investigation and reimplement accepted findings through a normal Slice. Do not use `agent/*`.

## Starting work

Create an independent Slice directly from current `develop`:

```bash
git switch develop
git switch -c slice/direct/<slice>
```

For a coordinated milestone, create a Wave from current `develop`, then create each Slice from its Wave:

```bash
git switch develop
git switch -c wave/<wave>

git switch wave/<wave>
git switch -c slice/<wave>/<slice>
```

Create a Task branch from its Slice only when concurrent workers need isolation. A worker that can safely operate in the Slice worktree returns evidence without adding a branch layer. Task commit history is working context; the slice planner reconciles Task results into the reviewable Slice outcome.

Before parallel dispatch, the slice planner or wave coordinator must confirm:

- dependencies are complete;
- decision ownership does not overlap concurrent work;
- mutable write leases are disjoint or an allowed exception is explicit;
- the coordinator and reviewers have capacity;
- every Slice has an independent completion gate; and
- parallel sibling Waves declare their Join Wave.

There is no fixed concurrency limit. Stop new dispatch when review queues, unresolved conflicts, or contention hotspots accumulate.

## Coordination contracts

Coordination artifacts link to accepted design owners. They do not copy or replace design authority. Start with the following minimum fields and amend them only from evidence gathered in W0.

### Wave Charter

```text
Goal:
Accepted decision references:
Stable boundaries:
Slice dependency graph:
Shared ownership:
Exit gate:
Out of scope:
```

### Slice Contract

```text
Observable outcome:
Owned contract:
Allowed write-set:
Dependencies:
Tasks:
Happy-path evidence:
Failure-path evidence:
Known and unverified cases:
Risk:
Required review lenses:
Omitted lenses and rationale:
```

### Task Brief

```text
Objective:
May read:
May write:
Must not decide:
Required evidence:
Return shape:
```

Wave Charters and Slice Contracts are human-reviewable branch artifacts. Task Briefs are transient dispatch metadata, not durable design documents. After acceptance, move stable decisions and discriminating evidence to their existing owners, summarize the outcome in the accepted commit, and discard the remaining coordination context. Do not retain completed coordination artifacts on `develop` by default.

Exact storage, schema, and generation remain experimental until the W0 exercise is reviewed.

## Ownership and reconciliation

One public contract or design decision has one active owner; concurrent work never shares it. Workers must not silently expand scope, decide a shared interface, or edit outside the allowed write-set. Return a focused proposal and evidence when an out-of-scope decision is required.

A mutable file has one active write lease by default. Concurrent writes to the same file require named independent regions, no shared import, signature, or ordering change, and a declared integration owner. Cargo manifests, crate facades, shared enums, and registries remain exclusive. Serialize the work as soon as an exception stops satisfying these conditions.

Workers do not resolve merge conflicts. A contract or design conflict blocks integration until its decision owner reconciles the SOT and affected Slices are reviewed again. Only a mechanical conflict goes to a neutral fresh-context integrator with both original contracts in view.

Repeated conflicts or files modified by many Tasks are contention hotspots. Pause parallel work and review ownership or module boundaries instead of adding workers.

## Test code

Treat a test as an executable explanation of one behavioral contract. Its name
and nearby explanation must let a reviewer understand:

- the scenario and relevant precondition;
- the observable result the assertions establish; and
- why that result protects a contract or failure boundary.

Do not claim behavior that the test does not observe. Distinguish values that
the test compares directly from inferred state, evidence, or implementation
detail. Cover the happy path and discriminating failure paths at contract
boundaries; record environment-dependent cases separately instead of presenting
them as internally verified.

Every built-in Rust test in the workspace must have an understandable Korean
line-comment immediately above `#[test]`. Use multiple lines when one line would
hide the setup, expected result, or reason. Do not merely translate the function
name or use unexplained jargon. `cargo xtask check test-explanations` enforces
presence; review owns factual accuracy and readability.

Keep small unit tests beside the private behavior they exercise. When fixtures,
failure matrices, or distinct behavior groups make the production module hard
to scan, move tests into a named `tests` module or integration-test files.
Choose the boundary by responsibility, not an arbitrary line-count threshold,
and keep shared test support separate from individual scenarios.

## Review and integration

Each Slice must include its implementation or docs, discriminating validation, public-contract updates, and known limits.

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

Use Codex in a separate fresh-context session for agent-performed independent
review by default. Use Kimi when the review specifically needs a different
perspective to search for hidden assumptions, counterexamples, credible
alternatives, or future costs. A Kimi request MUST ask for the strongest
counterargument before its verdict instead of asking it to confirm the
implementer's conclusion.

Every agent reviewer MUST receive the exact final diff, relevant authority,
requested lens, and validation evidence. If Kimi cannot start or finish its
different-perspective review because it is unavailable or its usage allowance
is exhausted, a separate fresh-context Codex session MAY retry the same
explicit lens. An unavailable default Codex reviewer is not retried until its
availability state changes. The implementing session's self-check is not an
independent review. A human may perform the exact review at any point.

If no agent or human reviewer completes the lens, mark the lens **unreviewed**
in the Slice status or handoff, identify each attempted reviewer and the
concrete availability reason, notify the human, and stop before acceptance or
integration. Do not treat quota exhaustion, reviewer unavailability, a partial
response, or an implementing-agent self-check as a completed lens. Do not
repeat an unavailable reviewer until its availability state changes. Failed
attempts are operational evidence, not accepted-commit trailers.

The accepted commit records completed lenses as evidence, not as a substitute
for performing them:

```text
Slice-Review: fresh-context - completed - <reviewer-id> - <clear|resolved>
Slice-Review: code-quality - completed - <reviewer-id> - <clear|resolved>
Slice-Review: integration - completed - <reviewer-id> - <clear|resolved>
```

Use `clear` when the completed review found no actionable findings. Use
`resolved` only after findings were addressed and the same lens re-reviewed the
final diff with no remaining actionable finding. Reviewer IDs are compact
tokens such as `kimi/session-id`, `codex/session-id`, or `human/name`; put
operational detail in the Slice status rather than free text in the trailer.

When no additional lens applies, record `Slice-Review: none - <reason>`.
`cargo xtask check slice-review-impact` fails closed when this disposition is
missing. It conservatively requires fresh-context review for product and tool
code, build and Cargo metadata, workflow authority, and semantic SOT authority,
it requires code-quality review for executable source under `crates/` and
`tools/` plus Developer Docs theme source, and it requires integration review
on a Wave branch. It reads only the Git trailer block. For a clean-index
message amend, it conservatively rechecks the current commit's paths. Path
detection is a minimum safety net: a planner MUST add any semantic lens that
the changed paths cannot discover. `none` cannot be combined with completed
lenses. A required lens counts only when its trailer uses the exact
`<lens> - completed - <reviewer-id> - <clear|resolved>` shape. Unavailable,
unfinished, pending, or unresolved reviews cannot satisfy a required lens.
This grammar is prospective: existing accepted commits retain their historical
trailers, while every new commit or amend after this change uses the new shape.

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

## Knowledge retention

At Slice or Wave close:

- move stable contracts and rationale to their existing decision owner;
- add short, repeated operational knowledge to this workflow authority;
- discard Task-local context and facts already expressed by code.

Do not preserve full agent transcripts by default. Measure progress by accepted Slices, validation coverage, unresolved conflicts, unverified cases, and executable milestone evidence—not commit, Task, or line counts.

`AGENTS.md` remains an authority index. Do not copy workflow rules or Wave logs into it.

## Local checks

Install the version selected by `hk.pkl`, then register its repository-local hooks:

```bash
cargo install hk --version 1.52.0 --locked
cargo install mdbook --version 0.5.4 --locked
hk install
```

`hk.pkl` owns the hook set. `hk check` verifies changes without editing them;
`hk fix` applies available fixes. Git `pre-commit` runs repository checks.
Repository-specific structured checks live in `tools/xtask`; `hk` invokes them
through the repository-local `cargo xtask` alias instead of placing development
policy in the `yo` product CLI.
For every accepted review commit, Git `commit-msg` requires the Slice review
disposition described above. Working commits on `slice/*`, `task/*`, and
`spike/*` defer it to their accepted squash or review commit. A Wave merge that
brings a commit already reachable from current `develop` into the Wave is
exempt because its component commits were already reviewed. Other merges into a
Wave, and every merge commit on `develop` or `main`, are not exempt and do not
replace the required squash or fast-forward workflow.

For accepted review commits on `develop`, `main`, or `wave/*` that change code
under `crates/` or `tools/`, delete code there, or change workspace Cargo
metadata or repository Cargo command configuration, Git `commit-msg` requires
exactly one trailer:

```text
Developer-Docs-Impact: updated
Developer-Docs-Impact: none - <why documented responsibilities and flows remain accurate>
```

`updated` requires a staged Developer Docs change. `none` requires a concrete
reason. The trailer forces the Slice review to consider documentation impact;
it does not claim that semantic accuracy can be decided automatically. Working
commits on `slice/*`, `task/*`, and `spike/*`, plus merge commits,
defer the decision to their accepted squash or review commit. Rerun `hk
install` after the repository adds or changes a hook event.

## History boundary

Treat `rib` as read-only. Keep audits, comparisons, and disposable prototypes in `.local-exclude/`; never force-add it. Track only independently rewritten and reviewed results.

Do not rewrite shared history or force-push without explicit approval.

## Integration readiness

Integrate any Slice only when:

- the diff contains one agreed outcome;
- declared dependencies are accepted and integrated;
- relevant tests, documentation checks, and `git diff --check` pass;
- tracked files contain no `.local-exclude/` content or `rib` copies; and
- risk, required review lenses, and any omission rationale are explicit; and
- the outcome, evidence, and limits are ready for review.

For a human-attention Slice, request approval only after this gate passes.
Explicit approval applies only to that Slice. Standing routine authorization
applies to eligible future Slices until the human revokes it. Follow-up work
still requires a new Slice, classification, and review disposition.
