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

For the resulting activation Slice, follow each structured `next_actions`
handoff in authority order instead of reconstructing the command chain from
memory. Save and reuse prepare results, refresh the registered ContextBuild
manifests, stage the exact returned transition paths, and only then run
`methexis check --staged-activation`. Prepare the final commit message,
including required review and Developer Docs impact trailers, before the first
commit attempt and run commit preflight against that exact message. Check lists
record executable commands with their required arguments, never placeholders.

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

Lease an exact file when the accepted responsibility is confined to that
file. When the same cohesive responsibility can reasonably require a new
sibling implementation, fixture, or focused test file, lease the narrowest
owning subtree with `/**` before work starts. Do not use a crate-wide or other
broad subtree merely to avoid coordination, and do not use a subtree lease to
absorb a second responsibility. This keeps ordinary module extraction and
finding resolution inside the reviewed ownership boundary without turning the
write-set into an open scope.

```bash
cargo xtask check slice-parallel <left.json> <right.json>
```

It requires the same current integration base—`refs/heads/develop` for direct
Slices or their `refs/heads/wave/<wave>` branch for Wave Slices—and rejects
overlapping write leases or contract ownership. It supplements the dispatch
checklist above. Bind each accepted contract once in its Slice worktree:

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

Before implementation, collect unresolved product and contract choices into
one alignment checkpoint with concrete effects and examples. Also classify the
required review lenses and reviewer routing; architecture and workflow changes
must either select or give a concrete reason to omit a different-perspective
reviewer. This classification does not start review: independent review begins
from the clean candidate defined under [Review and integration](#review-and-integration).
After the human accepts the exact contract and scope, continue without asking
again for the same decision.

Present a human checkpoint in this order: the concrete user-visible effect,
one representative example, the boundaries that remain unchanged, and the
exact decision requested. Put hashes, internal identifiers, and protocol terms
last. Record the accepted decision in the compact working note and do not
reopen it after continuation or context compaction unless new evidence changes
one of those effects or boundaries.

Return to the human only when new evidence creates a choice affecting product,
durable contract, failure behavior, compatibility, security, permissions,
destructive or external effects, or long-term ownership; when authorities
conflict; or when required validation or review is unclear. State the choice,
practical alternatives, and effects. Module boundaries, mechanical refactoring,
tests, diagnostics, and review fixes inside the accepted contract are not new
checkpoints: resolve them, update risk or routing when needed, and rerun the
affected lens.

### SOT-first changes

Before changing behavior that is governed by, contradicts, or deserves durable
SOT, make one narrow inventory of the affected implementation decisions and
their active Knowledge owners. Classify each decision as covered, missing, or
conflicting and record the required action. Read the active Checkpoint, those
Knowledge units, and the exact code anchors once; do not begin implementation
while a missing or conflicting contract remains unresolved.

Keep an evolving work procedure in this repository workflow authority and its
measurements in the local working note. Promote it to canonical Methexis
workflow Knowledge only after evidence from multiple completed Slices shows
that its semantics have stabilized and the human explicitly accepts the owner
transition. One task may reveal a workflow improvement without absorbing that
improvement into its delivery scope. Interrupt the delivery Slice only when the
current procedure cannot preserve correctness or authority; route a cost,
convenience, or tooling improvement to a separate governance Slice and finish
the accepted product path first.

Keep contract authority, Checkpoint activation, and implementation as separate
Slices. Until Methexis advertises the complete
`canonical-approval-on-demand-projection/v1` capability, use the legacy
sequence in which Source, Knowledge, and Korean Projection form one reviewed
candidate.

With that capability, use this order for a contract Slice:

1. author or revise only the decision Source and canonical English Knowledge;
2. run `methexis check --only relations` and complete the worker self-check;
3. commit one clean semantic candidate and perform each required review lens;
4. resolve all findings in one batch and reuse the same reviewer session for
   exact finding-resolution review when its lens and scope are unchanged;
5. after semantic review clears, prepare direct canonical approval for the
   exact English Knowledge revision by default;
6. only when the human explicitly requests additional Korean understanding,
   generate or reuse the matching Korean Projection, build the exact
   English-plus-Korean packet, and prepare the Projection-basis approval;
7. record exact human approval using only the prepared basis, stage the
   approval plus a Projection only for the Projection basis, and run
   `cargo xtask check methexis-check-for-stage`; and
8. integrate that proposal, activate it through a separate Slice, run ordinary
   full `methexis check` against the trusted activation, and implement through
   another separate Slice.

The approval-record path is the only required addition after semantic review;
the Projection path is additionally permitted only for an explicitly requested
Projection-basis review. Source, Knowledge, relations, and unrelated bytes
remain identical to the reviewed semantic candidate. Any semantic change
creates a new candidate and repeats the affected lens; a translation-only
correction repeats human review only. Working-diff review before the clean
candidate is preparation, not completed review.

Existing legacy Projection and approval records remain valid for their exact
approved revisions and are not regenerated in bulk. The capability selects the
current operation path and does not itself create durable authority lineage.

### Token-efficient agent operation

At session start, read each routed authority once, run `cargo xtask check
slice-scope`, and keep a compact working note containing only the base, allowed
write-set, current decision, exact candidate, validation state, and next
transition. Reuse that note instead of repeatedly loading broad history or full
documents. A continuation handoff contains these fields and unresolved work,
not transcripts or copied tool output.

Before marking a platform or external environment unverified, inspect the
registered local environment inventory and repository-owned runner entry
points. Use a compatible registered runner when its prerequisites and
authorization are present. Distinguish an absent registration, an unavailable
runner, and an executed failing check; only the first two are unverified. Add
every affected registered platform check to the candidate validation plan.
Before the full review packet, record each check as passed, failed, or
unverified instead of leaving it undiscovered; failed or unverified coverage
keeps the Slice from routine acceptance.

Use the narrowest evidence-producing command. Start with `rg`, `git status
--short --untracked-files=all`, `git diff --stat`, and targeted line ranges;
expand only when the result leaves a named uncertainty. Cap command output and
split a query before truncation. Do not treat `git diff` as including untracked
files. Avoid listing completed agents when a running reviewer identity is
already known; wait on or message that reviewer directly.

Parallelize independent read-only discovery and validation. Serialize Source,
Knowledge, Projection, approval, Checkpoint, activation, Git-index, commit, and
integration mutations in their authority order. Reuse hashes and manifests
returned by `author-revision` and the `prepare-*` commands; do not guess CLI
options, field names, revisions, model aliases, or replacement preconditions.
After a command fails, classify the failure and change the input or state before
retrying. Rerun a validation group only when its reviewed inputs changed or at a
declared Slice gate.

After the candidate is a clean commit, consolidate its existing evidence with
`cargo xtask slice gate <request.json>`. The transient, versioned request names
the candidate commit, planner-required review lenses, bounded validation
summary files, final review response files, known unverified environments,
risk classification, and any human-origin approval. Every validation and
review entry carries the same candidate commit; every review and exact approval
also carries the canonical base-to-candidate diff hash. Each referenced file is
a bounded regular file with an exact `sha256:<hex>` hash.

Do not copy those identities and hashes by hand when the candidate already has
a published review-chain manifest. Prepare and evaluate the gate request from
the existing artifacts instead:

```bash
mkdir -p .local-exclude/coordination/<slice>/validation
bash tools/validation/bounded-run.sh \
  --summary-out .local-exclude/coordination/<slice>/validation/xtask.json \
  xtask -- cargo test --locked -p xtask
```

`--summary-out` atomically creates a file byte-identical to the one-line
stdout summary. Its parent must already exist, and an existing target stops
before validation rather than being overwritten or causing a duplicate run.
Include that file as validation evidence in the immutable review packet; the
review-chain manifest and gate preparation then derive its path and hash.
The option changes storage only: it neither changes
`yo.validation-run-summary/v1alpha1` bytes nor discovers or reuses a prior
result.

Then prepare the gate request:

```bash
cargo xtask slice gate prepare <prepare.json> <gate.json>
```

```json
{
  "schema": "yo.slice-gate-prepare-request/v1",
  "manifest_path": ".local-exclude/methexis/slice-reviews/<id>/manifest.json",
  "validation_commands": [
    {
      "name": "xtask",
      "argv": ["cargo", "test", "--locked", "-p", "xtask"],
      "reused": false
    }
  ],
  "review_runs": [
    {
      "source": {
        "kind": "delivery_receipt",
        "receipt_path": ".local-exclude/coordination/<slice>/delivery.json",
        "class": "model-high"
      },
      "result_path": ".local-exclude/coordination/<slice>/review.txt",
      "verdicts": [
        {"lens": "fresh-context", "verdict": "clear"},
        {"lens": "code-quality", "verdict": "clear"}
      ]
    }
  ],
  "known_unverified_environments": [],
  "risk": {
    "classification": "human-attention",
    "rationale": "changes workflow authority"
  },
  "approval": null
}
```

The compact `yo.slice-gate-prepare-request/v1` input names the review-chain
manifest, each reviewed validation name with its original command and reuse
disposition, each final response with its lens verdicts, risk, and optional
approval. A model review source should name its
`yo.external-review-delivery-receipt/v1` and declare only whether the selected
route satisfies `model` or `model-high`; the command derives provider, model,
Session, reviewer, candidate, diff, artifact hashes, and required lenses. A
human or repository-local agent review without a Provider receipt uses
`{"kind":"declared_route","route":"human/<identity>"}` or the exact recorded
model route. Delivery receipts remain local operational assertions rather than
Provider-authenticated proof, so the coordinator still owns their factual
accuracy.

Preparation replays the complete original-or-delta review chain, requires the
validation command names to match every and only its final validation
artifacts, checks the current bound contract and clean candidate, validates the
generated gate request, and re-reads all inputs before atomically publishing a
new-or-byte-identical output. The same invocation returns the evaluated gate
result. New `yo.validation-run-summary/v1alpha1` evidence binds the launch
`HEAD`, clean worktree state, exact `argv` hash, and complete-log hash; the gate
verifies those fields against the candidate and declared command. Frozen
`yo.external-operation-evidence/v1` artifacts whose reviewed name starts with
`external-operation/` also pass through preparation without a copied summary.
The gate revalidates their exact candidate, embedded command arguments,
expected and observed exit status, counterfactual, and before/after
observations against the prepared command. They always require `reused: false`;
a successful match is a passed external-operation result and does not invent a
validation log path. Frozen
`yo.validation-run-summary/v1` evidence remains accepted for compatibility but
does not record its launch arguments, so its coordinator-supplied exact `argv`
remains a declaration rather than launch proof. When exact human approval is
later recorded, add only its compact kind, authority, and scope to the
preparation input and publish to a new output path; the command supplies the
approved candidate and diff. It never executes a check, sends a review,
interprets response prose, creates approval, commits, or integrates.

The gate does not execute validation, publish a review, interpret review prose,
grant approval, commit, or integrate. It verifies the bound Slice and clean
`HEAD`, derives the minimum path-based lenses from the ordinary review-impact
rules, rejects stale identities or changed evidence, and returns one bounded
JSON result whose `next_action` is exactly one of `validate`, `review`,
`approve`, or `integrate`. Its `commit_trailers` are usable only when the
corresponding recorded verdicts are factually accurate. The coordinator still
owns completeness of the declared validation plan, semantic lenses, risk
classification, and review disposition; the gate prevents identity drift, not
false statements.

An empty or failed validation set yields `validate`; missing required lenses
yields `review`; otherwise missing authorization yields `approve`. A
human-attention candidate requires `exact_candidate` approval bound to its
commit and diff. A routine candidate may instead cite a `standing_routine`
authorization with human origin and scope, but it cannot retain a known
unverified environment. Only a complete, green, reviewed, and authorized
request yields `integrate`. Keep the request and referenced evidence outside
tracked paths, and discard them with completed Slice coordination.

When the gate returns `integrate`, keep the semantic commit title, explanation,
and `Developer-Docs-Impact` decision in a small message source, but do not copy
`Slice-Review` or `Review-Coverage` lines into it. From the clean Slice
worktree, derive those exact trailers directly from the unchanged ready gate:

```bash
cargo xtask slice commit prepare \
  .local-exclude/coordination/<slice>/gate.json \
  /tmp/<slice>-message-source \
  /tmp/<slice>-commit-message
```

The command re-evaluates the gate, requires `next_action: integrate`, and
atomically publishes a new-or-byte-identical complete message. It does not
stage, commit, integrate, or approve anything. After the exact Slice diff is
squashed onto its integration branch, the existing
`cargo xtask slice commit /tmp/<slice>-commit-message` boundary still performs
preflight and creates the accepted commit.

Treat this `next_action` as the sole Slice-disposition prompt. Do not ask the
human for setup, validation, working-commit, review, staging, or integration
approval while an earlier gate action remains. When the gate first returns
`approve`, present one exact proposal naming its candidate commit, diff hash,
risk, and covered effects. A concise affirmative reply to that immediately
preceding unchanged proposal is sufficient; never require the human to copy or
retype the generated wording. Record that response as the request's
human-origin approval, rerun the same gate, and do not ask again when it returns
`integrate`.

For a direct Slice, one exact proposal may cover squash into the bound
integration ref, creation of its accepted commit, a normal fast-forward push
of that ref to its configured origin, and hash-bound Slice close cleanup. The
approval covers only the named candidate, diff, and effects. Any candidate or
effect change returns to the affected gate; force-push, destructive recovery,
additional semantic edits, and external Provider or validation-host egress are
never implicit in this scope. A routine Slice already covered by a recorded
`standing_routine` authorization proceeds from `integrate` without a
candidate-specific prompt. Platform capability or sandbox confirmation is a
separate execution boundary, not another Slice disposition; use only an
already granted narrowly scoped command capability and never fabricate or
broaden one from repository evidence.

If a required full suite fails outside the changed boundary, run each exact
failing test once in isolation to classify timing or shared-load sensitivity.
An isolated pass is diagnostic evidence, not a replacement for the failed
gate: rerun the original required suite and count both suite attempts and the
classification runs in the Slice close metrics. Do not loop retries or weaken
the assertion to manufacture a pass.

Before publishing immutable review input, finish the coordinator's actual-code
check and freeze the candidate, fixture prerequisites, planned Developer Docs
impact, review questions, and uniquely named validation evidence. Run
review-packet preflight to expose the exact managed-payload budget and section
costs without publishing an eligible packet. Publish only after the preflight
is ready and no input is expected to change; preflight is preparation, not
completed review.

Select review Knowledge anchors from the current active Checkpoint before
constructing the request. A known Knowledge file, earlier packet, or previously
active revision is not evidence that the anchor is active now. Resolve an
inactive anchor before packet publication instead of increasing the budget or
rebuilding broad context around it.

Treat preflight section costs as a Slice-sizing gate, not only a protocol
limit. When the Git diff dominates a packet that is no longer human-reviewable
or fits only by raising the configured budget, split the outcome at independent
responsibility and failure boundaries before publication. Do not raise a
packet budget merely to preserve an oversized Slice. An indivisible exception
must name why a smaller accepted outcome cannot preserve the contract and must
receive preliminary human authorization for that sizing exception before
external review. The completed exact Slice still receives its ordinary final
human-attention disposition after review and integration readiness.

At the first ready preflight, record why the configured maximum is appropriate
for the selected review route. Do not select or increase a maximum merely to
avoid a sizing gate. When managed payload is at or above 80% of that maximum,
or a later request increases the maximum, write a compact split assessment in
local Slice coordination before publication. Separate fixed ContextBuild and
repository-authority costs from candidate-specific diff and evidence costs,
estimate the plausible responsibility and failure-boundary splits, and record
the decision.

For this assessment, a material reduction is at least the greater of 2,000
managed tokens or 10% of the configured maximum. Split when an estimate
reduces total reviewer payload by that amount, or reduces the largest required
packet by that amount while increasing total payload by less than that same
amount, without breaking the accepted outcome. Remaining whole is also valid
when measured section costs show that no plausible responsibility split meets
either comparison because it would mostly duplicate fixed input; record the
estimated before-and-after values instead of requesting unnecessary human
authorization. An indivisible candidate-specific sizing exception still
requires the preliminary human authorization described above. The threshold
is an early-warning gate, not permission to reduce required authority, diff,
evidence, or review lenses.

Before publishing a full review packet that requires the split assessment
above, freeze a compact checklist of discriminating invariants in its review
questions. Each invariant must distinguish the accepted outcome from a
plausible counterexample and be decidable from the packet's included evidence;
do not restate the diff or request an open-ended repository audit. End the
checklist with an identity-bound stop condition: after the reviewer verifies
the exact packet identity, evaluates every declared lens against every listed
invariant, and resolves or reports all material findings, it returns the
verdict without unrelated repository inspection or repeated evidence
reconstruction. The stop condition never suppresses a new material finding or
a tool call needed to decide a listed invariant; it bounds only work after the
packet has already supplied sufficient evidence.

Run the declared complete candidate baseline after the last implementation
change and before publishing its full review packet. A finding-resolution
candidate may reuse unaffected evidence and rerun only affected checks for its
delta review, but it must pass the complete Slice-close baseline before the
accepted squash. Never defer that final-candidate baseline until after
integration or push.

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

At an orchestration handoff, assert the exact authoritative value consumed by
the next effect. A parallel copy, redundant callback argument, or collection
count does not prove that an opaque prepared object contains the same value. If
that object otherwise hides the boundary, expose the narrowest read-only,
secret-free typed projection needed for the assertion or exercise the real
downstream effect.

At an incremental resource or capacity boundary, identify the first unit that
would cross the limit and assert the contract's boundary result before
retention or emission. A rejecting boundary must reject it; a backpressure
boundary must leave it unaccepted or pending until capacity is available,
using the finite deadline and cleanup rules below. Compare the applicable
retained-state projection and emitted sequence; a later final-size check alone
does not prove incremental enforcement.

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

Validation code that coordinates a terminal, pipe, socket, child process, or
external runner must place a finite deadline on every potentially blocking
operation, including reads, writes, connection setup, accept, waits, and prompt
synchronization. An outer command or test timeout does not bound an inner
blocking operation. Consume protocol output required for peer progress before
observing dependent state, and make timeout cleanup terminate, reap, join, and
restore every resource the fixture owns.

## Review and integration

Each Slice must include its implementation or docs, discriminating validation, public-contract updates, and known limits.

### Required lenses

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

### Agent review protocol

After committing a clean candidate and saving declared validation output as
bounded evidence files, check the same versioned request before running
ContextBuild or packet measurement:

```bash
cargo xtask slice review-packet --check-readiness <request.json>
```

Readiness requires the exact bound Slice branch, base, contract, and clean
candidate; a direct ContextBuild request path inside that candidate worktree;
and unique, readable authority, validation, KnowledgeId, lens, and question
inputs. It final-revalidates those inputs before returning. It does not run
ContextBuild, capture the diff, tokenize a packet, create a ReviewId, or publish
an artifact. Methexis still owns and validates ContextBuild request semantics
when resolution starts. A green readiness result covers only these input
boundaries; it is preparation, not review evidence.

When correctness depends on behavior owned by an external executable rather
than the repository's typed seam—such as Git hook source selection, process
exit, signal, timeout, or filesystem publication ordering—include at least one
validation item named `external-operation/<portable-label>`. Its path must be a
JSON artifact with this shape:

```json
{
  "schema": "yo.external-operation-evidence/v1",
  "candidate_commit": "<full-candidate-commit>",
  "operation": {
    "working_directory": ".",
    "argv": ["git", "commit", "--amend", "--file", "message"],
    "expected_exit": {"kind": "code", "value": 1},
    "observed_exit": {"kind": "code", "value": 1}
  },
  "counterfactual": "The amend must fail before changing HEAD.",
  "observations": [
    {
      "name": "HEAD",
      "expected_relation": "equal",
      "before": "<full-before-commit>",
      "after": "<full-before-commit>"
    }
  ]
}
```

Record the working directory and use an argv array, not a shell transcript.
Exit kinds are `code`, `signal`, or `timeout`; observations use `equal` or
`different` and record explicit before/after values. Readiness requires a
well-formed, internally consistent artifact bound to the exact candidate,
reports how many such items it froze, and revalidates the same bytes before
returning. It does not execute arbitrary commands or prove the author ran them.
Root self-check and independent review still own whether the chosen
counterfactual discriminates the claimed failure boundary. A finding-resolution
delta may explicitly reuse unchanged evidence; an affected
`external-operation/*` item must bind its replacement candidate.

After readiness succeeds, inspect the exact candidate and run the
non-publishing preflight from that unchanged request:

```bash
cargo xtask slice review-packet --preflight <request.json>
```

Keep the named ContextBuild request inside the candidate Slice worktree;
Methexis rejects a request path that escapes that repository before resolving
any context.

The preflight captures and final-revalidates the same request, contract,
ContextBuild, authorities, validation evidence, and base-to-candidate diff used
by publication. It returns the prospective ReviewId, exact complete-packet byte
and token totals, maximum budget, and independently measured content/rendered
cost for each section. Section token counts are independently tokenized,
non-additive diagnostics; never sum them, and use the complete-packet token
count as the authoritative budget value. Preflight writes no eligible packet or
manifest. Any changed input invalidates the result; do not cite it as review
evidence.

For `yo.slice-review-markdown/v1alpha2`, preflight also reports an exact
content-addressed input prefix ending after the repository-authority sections.
Its standalone token count is another non-additive diagnostic, not a cached
token count. Matching prefix bytes create only a cache opportunity; record an
actual cache hit only when the provider exposes matching runtime metrics.

A later independent Methexis activation may use the experimental request
schema `yo.slice-review-packet-request/v1alpha3` with delivery profile
`yo.slice-review-markdown/v1alpha3`. It must add `activation_request_path`
naming the exact activation request inside the clean candidate worktree. The
packet resolves a review-only prospective ContextBuild, binds that request,
the proposed immutable Checkpoint, the proposed active record, its canonical
predecessor, and the trusted `develop` basis, and labels every result and
manifest `prospective`. It grants no approval, activation, or general context
eligibility. Do not infer an activation request or fall back to active
authority when any proposal input disagrees.

The change that introduces or modifies this prospective packet path must use
the already accepted ordinary review protocol. It cannot use the new path to
review its own enabling contract or implementation. Its first eligible use is
a later, independently prepared activation candidate; after that candidate is
integrated, ordinary full Methexis validation must still establish trusted
active authority.

The implementation enforces that bootstrap boundary: trusted `develop` must
contain the exact versioned capability record, and the candidate diff must be
the closed activation-only transition (active record, one immutable
Checkpoint, and the complete registered ContextBuild manifests). A missing or
different trusted capability, or any implementation, workflow, documentation,
or unrelated candidate path, requires the ordinary review protocol.

When the preflight is ready and its inputs are frozen, build the
content-addressed review input:

```bash
cargo xtask slice review-packet <request.json>
```

The request names the ContextBuild request and required included KnowledgeIds, exact Slice Contract,
repository-authority paths not carried by that build, validation evidence,
review lenses and questions, experimental `yo.slice-review-markdown/v1alpha2`,
`o200k_base/v1`, and the maximum managed-payload tokens. The command derives
the base from the bound Slice Contract and the candidate from clean `HEAD`,
captures a no-renames binary diff, and returns only the immutable packet and
manifest paths, hashes, ReviewId, and token count. An over-budget packet fails
without truncation. Deliver the exact `packet.md` bytes as the provider's sole
caller-controlled prompt; do not add parallel instructions or authority.

The current experimental profile renders the ContextBuild and repository authorities
under stable logical wrapper paths before the candidate-specific plan,
contract, evidence, instructions, and complete diff. Its manifest binds the
exact prefix boundary, hash, and standalone token count while preserving all
physical input paths in the complete plan and manifest. It never emits a
prefix-only or reference-only review. Frozen `yo.slice-review-markdown/v1` and
the superseded experimental `yo.slice-review-markdown/v1alpha1` requests and
manifests remain supported for exact in-flight reproduction and may root the
same unchanged review-delta v1 chain. New requests use `v1alpha2`; a published
profile identifier is never reinterpreted in place. `v1alpha3` is reserved for
the explicit prospective-activation request above and does not replace the
ordinary `v1alpha2` route.

Start every new experimental wire schema, profile, identity domain, request,
result, manifest, or receipt family at `v1alpha1`. Do not publish a new shape as
`v1` merely because it has no predecessor or only repository-local consumers.
Promotion from `v1alphaN` to stable `v1` is a separate reviewed compatibility
decision after producer, verifier, failure ordering, and migration behavior are
accepted. Existing `v1` identifiers remain frozen and are not renamed. When an
existing family needs changed experimental semantics, preserve that `v1`
behavior and add the smallest unused `v1alphaN`; reserve a stable incompatible
successor such as `v2` for a later explicit promotion rather than using it as an
experimental starting point.

Treat every published wire identifier as a frozen behavior boundary, not only
as a serialization label. If validation or failure semantics change, keep the
old producer/verifier behavior in its existing version-owned module and add the
smallest required `v1alphaN` module; do not reinterpret the old identifier or
jump to v2 while the contract remains experimental. The facade and verifier
must dispatch explicitly from the recorded schema/profile, while shared
capture and rendering code stays version-neutral. A compatibility test must
replay a discriminating artifact accepted by the frozen version and show that
the new version applies only its own stronger rule.

When the human has granted a standing external-review authorization, check it
against the exact published packet before asking for another egress approval:

```json
{
  "schema": "yo.external-review-standing-authorization/v1",
  "authority": "human/<owner>",
  "status": "active",
  "routes": [
    {
      "provider": "<provider>",
      "account": "<account>",
      "model": "<model>",
      "max_packet_bytes": 1000000,
      "max_managed_payload_tokens": 200000,
      "allow_original_fresh": true,
      "allow_finding_resolution_resume": true
    }
  ]
}
```

Keep this human-origin record outside Git at the one common-workspace path
`<Git-common-dir-parent>/.local-exclude/authorizations/external-review.json`.
Every worktree reads that current canonical file; copies and caller-selected
paths are ineligible. Create, replace, activate, or revoke it only from an
explicit human statement that names the exact routes and limits; a
Slice disposition, `go`, earlier delivery, or agent-authored proposal is not
standing egress authority. Removing the file or changing `status` from
`active` revokes it. The v1 semantics always exclude reviewer tool execution,
retry, steer, fallback, a second provider, and every additional provider
request. They permit at most one original packet in a fresh Session and one
direct finding-resolution packet resumed in that same reviewer Session. A
second delta, changed lens or scope, replacement route, unavailable-provider
substitution, or larger packet requires a new explicit human decision.

Bind one published manifest, the exact authorization bytes, route, and Session
mode in a transient request:

```json
{
  "schema": "yo.slice-review-egress-request/v1",
  "manifest_path": ".local-exclude/methexis/slice-reviews/<id>/manifest.json",
  "manifest_hash": "sha256:<manifest-hash>",
  "authorization_hash": "sha256:<authorization-hash>",
  "route": {
    "provider": "<provider>",
    "account": "<account>",
    "model": "<model>"
  },
  "session": {"mode": "fresh"}
}
```

After the original provider request durably starts, the coordinator records
the exact identity observed from Yo's durable StartTurn and Session evidence
before preparing a finding-resolution request:

```json
{
  "schema": "yo.external-review-delivery-receipt/v1",
  "review_id": "sha256:<original-review-id>",
  "packet_hash": "sha256:<original-packet-hash>",
  "route": {
    "provider": "<provider>",
    "account": "<account>",
    "model": "<model>"
  },
  "session_id": "<returned-reviewer-session>",
  "provider_request_id": "<returned-request-identity>",
  "provider_request_count": 1
}
```

This receipt is a local operational assertion, not authority. The preflight
proves only its exact bytes and internal consistency; it cannot authenticate
Provider provenance or prove the request count. The coordinator owns comparing
the fields with Yo's durable evidence and must not create the receipt before a
provider request starts. Bind its path and exact hash as `prior_delivery` in
the delta egress request, and use
`{"mode":"resume","id":"<same-reviewer-session>"}` only for that one direct
finding-resolution packet. The preflight rejects a different route, Session,
ReviewId, packet hash, absent request identity, or request count other than
one. Then run:

```bash
cargo xtask slice review-egress <request.json>
```

The command replays the complete original-or-delta manifest chain, verifies
the immutable packet, exact human-origin authorization hash, route, Session
mode, prior delivery receipt when required, byte and token limits, then replays
the complete chain again for final input and trusted-Git stability. It performs
no network or provider operation. `next_action: "deliver_once"` removes a
repeated human prompt only when the coordinator also observes that this exact
ReviewId, route, and Session step has no prior provider request. It is an
eligibility result, not a reusable delivery receipt: after a provider request
starts, record the returned Session/request evidence and never interpret
another preflight run as permission to resend. Terminal input that ended
before a durable provider request remains a delivery-system diagnostic and is
not silently retried under this authorization.

Before any manual terminal input for an authorized finding-resolution resume,
run the repository-owned read-only continuation preflight against the exact
durable Session repository:

```json
{
  "schema": "yo.slice-review-continuation-preflight-request/v1alpha1",
  "egress_request_path": ".local-exclude/coordination/<slice>/delta-egress.json",
  "egress_request_hash": "sha256:<exact-egress-request-hash>",
  "session_repository_path": "/absolute/path/to/the/reviewer-session-repository"
}
```

```bash
cargo xtask slice review-continuation-preflight <request.json>
```

The command replays the finding-resolution egress authorization, then reads
the named repository through `yo-core`. It requires exactly one StartTurn whose
bytes hash to the prior original packet, exactly one matching managed
Provider/Account/Model binding, exactly one accepted request and resumable
outcome resolving to the prior delivery receipt's request identity, and a
typed executable continuation with the newest durable Continuation Anchor. A
missing or malformed Session, mismatched route or identity, extra request, or
missing Anchor fails before terminal input. The successful result records the
exact Session, route, candidate, request identity, binding epoch, and Anchor
sequence, but publishes no artifact, acquires no terminal, and performs no
Provider request.

Run this preflight immediately before the manual input attempt and require that
no other process is writing the reviewer Session between observation and
delivery. Its result is current eligibility, not launch authority or retry
authority, and it never falls back to a fresh Session. The bounded
`review-deliver` command still owns only original-fresh delivery in v1alpha1;
extending repository-owned delivery to resume remains a separate effect change.

For one original packet in a fresh Session, perform the authorized effect with
the bounded repository delivery command instead of terminal paste, pane
capture, or direct Session JSONL inspection. Create one empty output directory
under the active Slice's shared coordination directory, then bind it and the
exact egress request in a new experimental request:

```json
{
  "schema": "yo.slice-review-delivery-request/v1alpha1",
  "egress_request_path": ".local-exclude/coordination/<slice>/egress.json",
  "egress_request_hash": "sha256:<egress-request-hash>",
  "output_directory": ".local-exclude/coordination/<slice>/delivery"
}
```

Run it once from the clean candidate worktree:

```bash
cargo xtask slice review-deliver <delivery-request.json>
```

The command replays `review-egress` authorization before and after preparing
the runtime, resolves the sole checked-out clean integration worktree at the
review manifest's trusted commit (including absence of non-ignored untracked
files), and builds that exact current-integration `yo`. It
then publishes an immutable `yo.external-review-delivery-claim/v1alpha1`
before process launch and pipes the already verified packet bytes directly to
one `yo -p --model <provider>:<account>:<model> --no-tools` process with a
30-minute outer deadline. It uses an isolated durable Session repository
inside the output directory and writes bounded `review.txt` and
`diagnostic.txt` files rather than copying packet, terminal, or raw Session
content into coordinator context.

After process completion, the command reads that isolated repository through
`yo-core`, requires exactly one byte-identical packet `StartTurn`, the exact
authorized managed Provider/Account/Model binding, one durable accepted
request, and one resumable Provider outcome. An outcome without its own stable
identity uses that accepted request identity, as required by the durable
Session contract. After a claim, setup, spawn, write, exit, timeout, capture,
publication, and durable-observation failures are folded into a compact
`yo.external-review-delivery-outcome/v1alpha1` whenever that output can still
be published; each result or diagnostic artifact says whether its bytes were
published. Only the fully successful path also publishes the frozen
`yo.external-review-delivery-receipt/v1` consumed by finding-resolution and
Slice-gate preparation, then returns `next_action: "interpret_review"`.

The claim is intentionally not idempotent: once it exists, the same output
directory can never authorize another launch, even when the process failed,
the terminal disappeared, or durable request evidence is incomplete. Inspect
the bounded outcome and request new human authority for any replacement; never
delete or rename the claim to manufacture a retry. Build and preflight failures
that occur before claim publication perform no Provider effect and may be
corrected normally. The initial v1alpha1 delivery path supports only an
original packet in a fresh managed-model Session. Finding-resolution resume,
delegated hosts, print continuation, retry, steer, fallback, a second Provider,
and tool execution remain outside this effect.

Because a self-hosting diff may contain the wrapper's own sentinel source,
v1alpha2 section metadata names its reversible sentinel-escape profile. The
encoding doubles literal backslashes and replaces the first less-than byte of
a literal wrapper-sentinel prefix with ASCII `\x3c`; hashes and byte counts
continue to bind the decoded original. This keeps in-body source text from
being mistaken for a real section or payload boundary.

Use a separate fresh-context Codex session by default. Use a configured
different-perspective provider when the lens needs hidden assumptions,
counterexamples, alternatives, or future costs; require its strongest
counterargument before the verdict. A provider is configured only after the
human selects it and local guidance defines invocation, stable identity, setup
failure, and unavailability. Kimi is the current preferred profile, not a
product dependency. Do not invent commands for an unconfigured provider.

Run a Kimi review from the Slice worktree in a new non-interactive prompt
session. The sentinel prevents command substitution from stripping the
packet's trailing newlines and is removed before invocation:

```bash
review_payload="$(cat <packet.md>; printf x)"
kimi -p "${review_payload%x}"
```

Every reviewer receives the immutable `<base>..<candidate>` diff, relevant
authority, requested lens, and validation evidence. Commit the complete
candidate and require a clean worktree; dirty or inferred review surfaces do
not count. Reviewers may rerun a targeted check to resolve a finding or named
uncertainty, but do not repeat a supplied green baseline when its inputs are
unchanged. If a finding changes the candidate, validate the affected boundary
and review the new commit again. When the lens, scope, and reviewer are
unchanged, resume that exact reviewer session and provide the prior candidate
and ReviewId, replacement candidate, exact delta bytes or their
content-addressed artifact, finding dispositions, and the unchanged evidence
the reviewer may reuse. The coordinator validates command syntax, artifact
identity, and supplied green checks before delivery; do not make the reviewer
rediscover those operational facts.

Build that provider-neutral continuation payload from the prior immutable
manifest and a clean replacement `HEAD`:

First record the exact finding set returned by that reviewer. This artifact is
reviewer-authored evidence, not a worker-selected subset:

```json
{
  "schema": "yo.slice-review-findings/v1",
  "review_id": "sha256:<prior-review-id>",
  "candidate_commit": "<prior-candidate-commit>",
  "findings": [
    {"finding_id": "F1", "summary": "The ambiguous state is accepted."}
  ]
}
```

```json
{
  "schema": "yo.slice-review-delta-request/v1alpha1",
  "prior_manifest_path": ".local-exclude/methexis/slice-reviews/<review-id>/manifest.json",
  "prior_manifest_hash": "sha256:<manifest-hash-from-review-packet-result>",
  "prior_findings_path": ".local-exclude/coordination/<slice>/review-findings.json",
  "prior_findings_hash": "sha256:<exact-findings-file-hash>",
  "finding_dispositions": [
    {
      "finding_id": "F1",
      "disposition": "resolved",
      "summary": "The replacement candidate now rejects the ambiguous state."
    }
  ],
  "reused_validation_evidence": ["unchanged-baseline"],
  "affected_validation_evidence": [
    {"name": "focused-finding-check", "path": "/tmp/focused-check.txt"}
  ],
  "delivery_profile": "yo.slice-review-delta-markdown/v1alpha1",
  "tokenizer_profile": "o200k_base/v1",
  "max_managed_payload_tokens": 12000
}
```

```bash
cargo xtask slice review-delta <request.json>
```

The command fully reproduces the prior packet and manifest from their captured
inputs before accepting them; the prior manifest may be either the original
review or the latest verified review delta in the same chain. It then requires dispositions (`resolved`,
`not_reproduced`, or `accepted_limit`) for every and only prior finding ID,
requires at least one replacement-specific affected validation item, requires
every prior validation item to be classified as reused or affected, verifies
reused bytes, requires every affected evidence body to name the exact
replacement commit, and enforces a cumulative evidence bound while capturing.
Store each candidate's evidence at a new immutable path; overwriting evidence
referenced by an earlier review makes that chain head ineligible. The command
captures the no-renames binary prior-to-replacement diff and returns one
content-addressed `packet.md` plus manifest. Deliver those packet bytes
unchanged to the recorded reviewer session. This command does not start or
select a provider.

Before replaying a published original review, its verifier validates every
manifest Git revision without invoking Git. It then requires the base and
candidate fields to name those exact commit objects and requires the base to
be an ancestor of the candidate before reading ContextBuild, authority,
validation, contract, or diff inputs. Malformed, tag-object, missing-object,
and unrelated-history identities therefore fail before input replay.

New continuation requests use experimental
`yo.slice-review-delta-markdown/v1alpha1`, which compares affected evidence by
canonical filesystem identity. Published delta-v1 manifests retain their
frozen path-string transition rule so an old alias-shaped chain remains
reproducible; accepting that legacy artifact does not permit a new v1 request.
The verifier selects these semantics from the exact published manifest schema.

The continuation payload and question are bounded. Policy does not impose a
finding-resolution round count, but the verifier has a 64-hop safety limit; at
that boundary, start a fresh review instead of extending the chain. Keep
reusing the session while it can identify the prior review and the
remaining work without broad reconstruction. Start a compact fresh session
when the lens or scope changes, the reviewer is unavailable, exact context is
lost, the reviewer begins broad repository or documentation reinspection, or
the next finding introduces a new design question instead of resolving the
reviewed one. Repeated tool calls that mostly reconstruct supplied evidence are
a signal that accumulated session context is no longer helping. A resumed
reviewer that cannot identify the prior packet fails closed and does not count
as completed review.

For Codex CLI review, resume the recorded session through its supported
non-interactive surface:

```bash
codex exec resume --json <session-id> - < <delta-review-packet.md>
```

Do not return a large reviewer's prompt echo or event stream to the coordinating
agent context. For full and delta packets, direct complete process output to a
task-specific local log, request the final message in a separate file when the
reviewer supports it, and read back only the exposed session identity, terminal
status, and final response. Preserve the exact reviewer-authored finding set
and verdict when findings exist so a later review delta can reproduce them;
discard prompt echoes and event or tool streams. The immutable packet and
manifest already own the review input, so copying them through orchestration
output wastes context and can terminate an otherwise valid review run. Preserve
the local log only while a finding remains unresolved.

For Kimi, use plain `kimi -p`: never `--continue` or `--session`. Omit `--model`
unless its exact configured CLI alias is known, and consult `kimi --help`
instead of guessing flags. Require empty `git status --short
--untracked-files=all` output before and after; reviewer-created changes
invalidate the attempt. Invalid options and aliases are setup failures to fix,
not unavailability. Record the completion hint's session as
`kimi/<session-id>`; without that hint, do not invent an identity or count the
attempt as complete.

For every completed agent review, record the lens, actual provider, exposed
model and session identity, and verdict in the Slice status or handoff; report
the actual provider and model at close. Record missing identifiers rather than
guessing. If a correctly invoked preferred provider cannot finish because the
service or allowance is unavailable, record the requested provider and reason,
then a separate fresh-context Codex session may perform the same lens. Do not
retry an unavailable provider until its state changes. Human exact review is
also valid; the implementing session's self-check is not.

When the provider exposes them, also record managed packet tokens, model-call
and reviewer-tool-call counts, cumulative input, cached input, output, and the
number of finding-resolution rounds. Managed packet size is not a proxy for
the total work of a tool-using review session. Use these measurements to compare
Slice workflows and accepted outcomes, not as a reason to weaken a required
lens or optimize token count in isolation.

If no reviewer completes a required lens, mark it **unreviewed**, record each
attempt and reason, notify the human, and stop before acceptance. Quota
exhaustion, partial responses, self-checks, and failed attempts are not review
trailers.

### Evidence and disposition

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
```

`model-high/<provider>/<model>/<session>` records an independently selected
high-capability model and is required for model-performed fresh-context and
integration review. Mechanical code-quality review may use
`model/<provider>/<model>/<session>`. The provider and session must reproduce
the compact `<provider>/<session>` identity in the matching `Slice-Review`.
This records the actual route without hard-coding one vendor as policy.

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

### Integration and cleanup

Run the declared baseline on the reviewed candidate. After squash, reuse its
result only under the exact conditions in the Developer Docs
[Slice-close baseline](docs/src/validation/README.md#slice-close-baseline);
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
effects, and the coordination entries that will remain. Generate a fresh plan
if integration advances. Newly generated plans use
`yo.slice-close-plan/v4`. A v2 or v3 plan remains resumable under its original
identity and safety checks only when its accepted commit predates the tracked
close-metrics cutover marker; commits containing that marker require v4 even if
a caller rewrites and rehashes a legacy-shaped plan.

`apply` revalidates that state and the retained-entry list, then removes only
the registered worktree and binding, the exact standard
`.local-exclude/coordination/<slice>/slice-contract.json` when applicable, and
the expected local Slice branch. It preserves remote refs, nonstandard
contracts, handoffs, requests, notes, and other reported coordination entries;
reconcile those through their own owner. The entire worktree, including ignored
output, is removed, so move retained material first. A repository lock, final
worktree check, and compare-and-swap ref transaction protect cooperating
applies; raw Git does not use the lock. A stale plan fails closed, while an
interrupted apply can resume the planned contract or branch deletion.

### Wave promotion

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

Inspect close metrics before applying cleanup. If the completed Slice exposes a
repeatable workflow correction that is not already owned here, promote only
that rule through a separate reviewed workflow Slice before deleting the local
metrics. Metrics and transcripts remain temporary evidence and are not copied
into durable history.

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
cargo xtask slice commit /tmp/yo-commit-message
```

The explicit preflight loads the staged impact once and reports both Slice
review and Developer Docs trailer failures before expensive `pre-commit`
checks run. The commit command repeats preflight, copies the exact prepared
message through Git's ordinary editor boundary, and lets Git `commit-msg`
repeat the same combined check as final enforcement. Direct `-m`, `-F`, and
template sources are rejected on accepted branches because Git otherwise
reports amend combinations under those indistinguishable source names. The
command therefore also fails with a focused diagnostic when `commit.template`
is configured; unset it for this commit path.

For accepted review commits on `develop`, `main`, or `wave/*` that change code
under `crates/`, `shared/`, or `tools/`, delete code there, or change workspace Cargo
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
