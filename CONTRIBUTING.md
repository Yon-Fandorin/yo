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

For a direct Methexis activation Slice, record only the semantic coordination
input and let `xtask` fix the mechanical boundary. From a clean `develop`
worktree, save a versioned local request such as:

```json
{
  "schema": "yo.activation-slice-request/v1",
  "slice": "tui-example-activation",
  "owned_contracts": ["tui.example.activation"],
  "dependencies": [
    "approved tui.example revision <revision>",
    "exact activation transition authorized by human/<owner>"
  ]
}
```

Then run:

```bash
cargo xtask slice create-activation <request.json>
```

The command pins the current `develop` commit; writes the canonical activation
contract under `.local-exclude/coordination/<slice>/`; creates
`slice/direct/<slice>` and its worktree under
`.local-exclude/worktrees/<slice>/`; and binds that contract to the worktree.
An exact retry reuses completed effects and restores a missing worktree or
binding. Conflicting refs, paths, contracts, or bindings fail closed. This
setup does not approve a revision, create a Checkpoint, propose activation, or
record human authorization.

The versioned JSON result reports the contract, branch, worktree, and binding
effects separately. A failed invocation returns the same result schema with
each effect observed as `prepared`, `absent`, `conflicting`, or `unknown`, so a
caller can save the exact partial state before retrying. If `develop` advances
after the contract was published, the same request retains that contract's
pinned base rather than silently changing the Slice identity.

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

Keep active coordination contracts outside Git, normally below
`.local-exclude/coordination/`. When concurrent Slices need a mechanical
preflight, represent only their coordination boundary as
`yo.slice-contract/v1` JSON:

```json
{
  "schema": "yo.slice-contract/v1",
  "slice": "tui-visual-polish",
  "base": "<full integration-base commit ID>",
  "base_ref": "refs/heads/develop",
  "owned_contracts": ["tui.visual-presentation"],
  "dependencies": ["active TUI appearance contracts"],
  "allowed_write_set": [
    "crates/yo-tui/src/appearance/**",
    "crates/yo-tui/tests/**"
  ],
  "focused_checks": ["cargo test -p yo-tui appearance"],
  "slice_close_checks": ["cargo test -p yo-tui", "hk check"]
}
```

The JSON is transient coordination metadata, not a second design authority.
Paths are repository-relative exact files or subtrees ending in `/**`; the
allowed write-set is closed, so every omitted path is forbidden. Commands in
the check lists are reviewable evidence declarations and are never executed
from the JSON. Before dispatch, run:

```bash
cargo xtask check slice-parallel <left.json> <right.json>
```

It is one mechanical preflight: both contracts must name the same current
integration base—`refs/heads/develop` for direct Slices or their
`refs/heads/wave/<wave>` branch for Wave Slices—and it rejects overlapping
write leases or contract ownership. The planner still confirms dependencies,
capacity, independent completion gates, and any Wave join required by the
dispatch checklist above. The planner then binds each contract once in its
Slice worktree:

```bash
cargo xtask slice-contract bind <its-contract.json>
```

The binding lives inside that worktree's Git metadata, outside repository
history. At the start of every new coding-agent session, run the following
before planning or editing:

```bash
cargo xtask check slice-scope
```

The command discovers and reports the bound contract. A missing or invalid
binding stops work instead of guessing a scope. Run the same check during
implementation and before review. It compares tracked, staged, working-tree,
and untracked changes since the declared base with the closed write-set. A
newly required shared path is a coordination decision, not a reason to widen
the contract silently. The explicit
`cargo xtask check slice-scope <contract.json>` form remains available for
one-off validation before a binding exists.

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

## Alignment and human checkpoints

Do not turn one Slice into a sequence of routine confirmation requests. Before
implementation, collect unresolved product and contract choices into one
alignment checkpoint with their concrete effects and examples. Once the human
accepts the exact contract and scope, continue through Slice setup,
implementation, validation, review fixes, and review without asking again for
the same decision.

Classify the required review lenses and reviewer routing during the initial
alignment, not for the first time after implementation. For architecture or
workflow changes, explicitly evaluate whether the different-perspective
reviewer criterion below applies and record a concrete rationale when it does
not. This classification selects the reviewer; it does not dispatch one early.
Independent review starts only from a clean, immutable candidate commit under
the review protocol below. A review of a dirty working diff is preparation, not
completed evidence.

If implementation reveals new impact, escalate risk and update the required
lenses or routing. That update is not another human checkpoint unless resolving
it requires one of the human-owned choices below.

Pause and return to the human only when new evidence creates a choice that can
change the product, durable contract, failure behavior, compatibility, security,
permissions, destructive or external effects, or long-term ownership; when the
accepted authorities conflict; or when required validation or review cannot be
made clear. State the newly discovered choice, its practical alternatives, and
the effect of each. Use the human-attention classification below for the final
integration disposition.

Implementation details that remain inside the accepted contract—module
boundaries, mechanical refactoring, test coverage, diagnostics, and fixes for
review findings—do not create another human checkpoint. Resolve them, rerun the
affected review lens, and continue. A reviewer finding becomes a checkpoint
only when resolving it requires one of the human-owned choices above.

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
review by default. Use a configured different-perspective reviewer when the
review specifically needs another model or provider to search for hidden
assumptions, counterexamples, credible alternatives, or future costs. Kimi is
the current preferred route, not a permanent product dependency. Such a request
MUST ask for the strongest counterargument before its verdict instead of asking
it to confirm the implementer's conclusion.

A different-perspective reviewer is configured only when the human has selected
the route and local operating guidance defines its invocation, stable review
identity, and setup-failure and unavailability handling. The Kimi-specific
protocol below is the current provider profile. Add or replace that profile when
the configured route changes; do not invent commands for a hypothetical
provider.

Run a Kimi review from the Slice worktree in a new non-interactive prompt
session:

```bash
kimi -p 'Perform a read-only independent review. Do not edit files.
Exact final diff: <base>..<candidate>.
Authority: <paths and owned contracts>.
Review lens: <lens and concrete questions>.
Validation evidence: <commands, environments, and results>.
Before the verdict, present the strongest counterargument and credible
alternatives. Return path-specific findings, the verdict, and unresolved
uncertainty.'
```

Replace every placeholder with exact immutable Git commits; do not ask the
reviewer to infer which changes are final. Commit the candidate on its Slice
branch before review and require a clean worktree so `<base>..<candidate>`
includes the complete review surface. Dirty staged, unstaged, or untracked
changes are not an exact review candidate. If findings change the candidate,
update its working commit and review the new commit range again.

Plain `kimi -p` starts the fresh prompt-mode session required here. Do not use
`--continue` or `--session`, because either reuses earlier context. Omit
`--model` unless the review contract requires a model whose exact configured
alias is known; a model name displayed by an interactive UI is not necessarily
a valid CLI alias. Consult the installed `kimi --help` rather than combining
prompt mode with guessed interactive or permission flags. Run
`git status --short --untracked-files=all` immediately before and after Kimi;
both results MUST be empty. Any reviewer-created change invalidates that
attempt and must be resolved before a new review starts.

An invalid option, unknown model alias, or other local invocation error is a
setup failure, not reviewer unavailability: correct the invocation and retry
the same review. A completed text-mode prompt ends with
`To resume this session: kimi -r <session-id>`; record that value as
`kimi/<session-id>`. If the hint is absent, do not invent an identity or count
the attempt as completed. Apply the Codex fallback below only when a correctly
invoked Kimi session cannot start or finish because the service or usage
allowance is unavailable.

Every agent reviewer MUST receive the exact final diff, relevant authority,
requested lens, and validation evidence. For every completed agent review, the
Slice status or handoff MUST record the requested lens, actual provider, exact
model identifier and reviewer session identity when exposed, and verdict. When
closing the Slice, report the actual provider and model to the human. If either
identifier is not exposed, record and report that fact rather than guessing.

If the preferred different-perspective reviewer cannot start or finish because
it is unavailable or its usage allowance is exhausted, a separate fresh-context
Codex session MAY retry the same explicit lens. When the fallback starts, record
the requested reviewer and concrete availability reason; after it completes,
record and report the actual fallback provider and model under the rule above.
An unavailable default Codex reviewer is not retried until its availability
state changes. The implementing session's self-check is not an independent
review. A human may perform the exact review at any point.

If no agent or human reviewer completes the lens, mark the lens **unreviewed**
in the Slice status or handoff, identify each attempted reviewer and the
concrete availability reason, notify the human, and stop before acceptance or
integration. Do not treat quota exhaustion, reviewer unavailability, a partial
response, or an implementing-agent self-check as a completed lens. Do not
repeat an unavailable reviewer until its availability state changes. Failed
attempts are operational evidence, not accepted-commit trailers.

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

After the accepted commit exists, close its local Slice in two explicit steps
from the integration worktree:

```bash
cargo xtask slice close plan <slice> > /tmp/<slice>-close.json
cargo xtask slice close apply /tmp/<slice>-close.json
```

Review the plan before applying it. `plan` requires clean integration and Slice
worktrees, the original bound Slice contract, valid review evidence on exactly
one accepted commit in the integration branch's first-parent history, and a
verbatim patch identity shared by the Slice and that commit. A later accepted
Slice does not strand an older local worktree; generate a fresh plan at the
new integration head. The hash-addressed plan fixes the integration and Slice refs,
worktree, binding, and effects. `apply` revalidates them immediately before it
removes only the registered worktree and its binding, then deletes only the
expected local Slice branch with a compare-and-swap ref update. It can finish
the branch deletion if interruption occurred immediately after worktree
removal. Store the plan outside the worktree it removes.

The removed worktree path is deleted in full, including ignored build output or
scratch files inside it. Move anything worth retaining out of that path before
apply. The command never targets remote refs, coordination contracts, handoffs,
notes, or other local artifacts outside the planned worktree. Reconcile or
discard those through their own owner after retaining any stable knowledge. A
stale plan fails closed; do not edit its JSON to make it pass. Cooperating
`slice close apply` processes share a repository lock. Raw Git processes do not
honor it, so the final worktree snapshot, Git's dirty-worktree refusal, and one
atomic ref transaction recheck the planned integration ref while deleting the
exact Slice ref.

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
Semantic SOT files and tool implementation use separate `hk` routes. SOT
changes run one staged Methexis integrity report plus the focused Librarian
live-corpus compatibility test; they do not rerun either tool's implementation
suite. Changes below `tools/methexis/` or `tools/librarian/` continue to run the
respective full suite. For a staged Methexis activation, the integrity route
reads one `check --staged-activation` report and reports prospective authority.
This is not a test exemption: after integration, ordinary `methexis check` and
the full Methexis tests are required against trusted `develop`.

At every Slice close, include a compact Methexis/Librarian token retrospective
in the Slice status or handoff. Use exactly one of these compact shapes:

```text
Knowledge-tool token retrospective: not invoked
```

```text
Knowledge-tool token retrospective:
- <tool> | trigger=<reason> | route=<narrowest command> | count=<n> | reuse=<hash/build or none>
- Next reduction: <one concrete change>
```

Group repeated calls that share the same tool, trigger, and route into one
counted line; add another group line only when one of those fields differs.
Never copy full payloads or tool output, and record exactly one next reduction
for the whole Slice. Do not promote this operational note into durable
repository history unless it changes a workflow or tool contract.

For every accepted review commit, Git `commit-msg` requires the Slice review
disposition described above. Working commits on `slice/*`, `task/*`, and
`spike/*` defer it to their accepted squash or review commit. A Wave merge that
brings a commit already reachable from current `develop` into the Wave is
exempt because its component commits were already reviewed. Other merges into a
Wave, and every merge commit on `develop` or `main`, are not exempt and do not
replace the required squash or fast-forward workflow.

When constructing an accepted commit message programmatically, write the
complete message to a temporary file and validate that prepared file before
invoking `git commit`:

```bash
cargo xtask check commit-preflight /tmp/yo-commit-message
git commit --file /tmp/yo-commit-message
```

The preflight loads the staged impact once and reports both Slice review and
Developer Docs trailer failures together. Git `commit-msg` repeats that same
combined check as the final enforcement boundary; the explicit preflight is
what catches message errors before expensive `pre-commit` checks run.

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
