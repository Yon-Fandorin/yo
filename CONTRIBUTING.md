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

## Review and integration

Each Slice must include its implementation or docs, discriminating validation, public-contract updates, and known limits.

Every Slice receives worker self-check and human review. The slice planner proposes its risk and required lenses in the Slice Contract; workers cannot lower them. Escalate risk when implementation reveals public-contract, failure-behavior, or shared-ownership impact. Treat uncertainty as higher risk, and let the human reviewer approve the final classification.

Fresh-context contract review is required for public contracts, terminal lifecycle, concurrency, failure behavior, workflow, and SOT changes. Slice integration review is required for Wave Slices and changes that consume shared interfaces or sibling results. A simple independent docs or configuration Slice may omit an additional lens only with a recorded rationale.

After explicit human approval, squash a direct Slice into `develop`:

```bash
git switch develop
git merge --squash slice/direct/<slice>
git commit
```

Squash an approved Wave Slice into its Wave:

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
hk install
```

`hk.pkl` owns the hook set. `hk check` verifies changes without editing them; `hk fix` applies available fixes. Git `pre-commit` runs checks only.

## History boundary

Treat `rib` as read-only. Keep audits, comparisons, and disposable prototypes in `.local-exclude/`; never force-add it. Track only independently rewritten and reviewed results.

Do not rewrite shared history or force-push without explicit approval.

## Merge gate

Request Slice approval only when:

- the diff contains one agreed outcome;
- relevant tests, documentation checks, and `git diff --check` pass;
- tracked files contain no `.local-exclude/` content or `rib` copies; and
- risk, required review lenses, and any omission rationale are explicit; and
- the outcome, evidence, and limits are ready for review.

Approval applies only to that Slice. Follow-up work requires a new Slice and review.
