# Contributing to yo

Make tracked changes as reviewable Slices. Add a Wave only when multiple Slices share an executable milestone, dependency graph, parallel join, or common exit gate. Add Task branches only when workers need isolation. This file and the authority pages it links are the repository workflow authority.

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

Compare the declared write-sets before freezing sibling candidates. If work
that will integrate in sequence shares a workflow, documentation, manifest, or
other mutable path, declare that dependency and create the downstream Slice
from the accepted parent instead of freezing both from the same base and later
recomposing their edits. Keep same-base parallelism for disjoint write-sets;
`cargo xtask check slice-parallel` remains the mechanical check.

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

Do not repeat a full semantic provider review solely because direct canonical
approval creates a new commit. The repository may carry the cleared review to
that strict descendant only when the reviewed candidate changed exactly one
Knowledge file matching the requested KnowledgeId, did not change its approval
path, and completed the fresh-context lens; the descendant may change exactly
`methexis/approvals/<KnowledgeId>.yaml`. Methexis must rederive the current
Knowledge revision, owner, request hash, replacement precondition, and exact
canonical approval bytes from Git. Any other descendant path, noncanonical
byte, stale revision, or owner mismatch requires a new affected review rather
than carry. Carry binds review evidence to an exact mechanically derived
candidate; it neither records nor grants human approval.

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

Once any immutable review manifest names that candidate, preserve it as an
ancestor: do not amend, rebase, or replace the reviewed commit. Apply a finding
fix as a new child commit and publish a review delta. `cargo xtask slice status
<slice>` checks every manifest bound to the exact Slice Contract and reports
`review_lineage:"broken"` when a reviewed candidate is no longer an ancestor
of current Slice `HEAD`; stop there instead of regenerating evidence for the
rewritten history.

Do not copy those identities and hashes by hand when the candidate already has
a published review-chain manifest. Prepare and evaluate the gate request from
the existing artifacts instead:

```bash
mkdir -p .local-exclude/coordination/<slice>/validation
bash tools/validation/bounded-run.sh \
  --summary-out .local-exclude/coordination/<slice>/validation/xtask.json \
  --reusable-local \
  xtask -- cargo test --locked -p xtask
```

`--summary-out` atomically creates a file byte-identical to the one-line
stdout summary. Its parent must already exist, and an existing target stops
before validation rather than being overwritten or causing a duplicate run.
Include that file as validation evidence in the immutable review packet; the
review-chain manifest and gate preparation then derive its path and hash.
`--summary-out` changes storage only. `--reusable-local` is a separate,
explicit assertion that the command is deterministic from local repository
bytes and has no network, clock, account, service, or other external-state
dependency. It emits `yo.validation-run-summary/v1alpha3`; omit it to retain
the frozen `v1alpha2` output.

Do not rerun an unchanged passing command merely to give a descendant candidate
a new filename. When the review-delta transition classifies that exact summary
as unaffected, keep its original path and hash, list it under
`reused_validation_evidence`, and set the matching gate preparation command to
`"reused":true`. Run a new bounded command only for affected validation or the
declared final Slice-close baseline. The immutable delta chain and gate, not a
copied summary, prove that reuse applies to the descendant.

Use this bounded runner for every coordinator-owned validation command,
including the final combined-workspace gate. Read and retain the one-line
summary by default; open the bounded log only when the summary reports failure
or a named diagnosis requires it. Do not paste successful compiler or test
logs into coordinator or reviewer context.

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

Use the delivery request's `v1alpha4` form when the completed review must also
bind actual Provider Usage. It preserves the existing claim, outcome, and
`delivery.json` schemas and publishes a separate
`yo.external-review-provider-usage/v1alpha1` artifact at
`provider-usage.json`. The artifact independently reopens the durable Session,
requires the delivery receipt's exact request identity on its recorded turn,
and requires every usage source on that turn to match the admitted managed
Provider/Account/Model or delegated Codex/Grok host. It records the Session and
turn identities, source-specific receipt fields, the hash and byte count of
each raw terminal usage snapshot, and presence-aware input, output, total,
reasoning, cache-read, and cache-write values. Multiple receipts are summed
with explicit complete or partial coverage. An absent receipt is represented
as `unavailable`, never as a guessed zero. A request, Session, turn, target, raw
snapshot, or usage projection mismatch fails the already-claimed attempt
without launching another Provider request. Earlier request and result
versions retain their existing output bytes; only `v1alpha4` returns the new
result version containing the content-addressed Provider Usage artifact.
Select it through `yo.slice-review-prepare-request/v1alpha3`; older preparation
versions continue to emit their frozen delivery versions.

For a review prepared with `yo.slice-review-prepare-request/v1alpha2`, use the
structured gate-preparation shape and omit caller-declared verdicts:

```json
{
  "schema": "yo.slice-gate-prepare-request/v1alpha2",
  "manifest_path": ".local-exclude/methexis/slice-reviews/<id>/manifest.json",
  "validation_commands": [{
    "name": "xtask",
    "argv": ["cargo", "test", "--locked", "-p", "xtask"],
    "reused": false
  }],
  "review_runs": [{
    "source": {
      "kind": "delivery_receipt",
      "receipt_path": ".local-exclude/coordination/<slice>/delivery.json",
      "class": "model-high"
    },
    "result_path": ".local-exclude/coordination/<slice>/review.txt"
  }],
  "known_unverified_environments": [],
  "risk": {
    "classification": "human-attention",
    "rationale": "changes workflow authority"
  },
  "approval": null
}
```

This experimental shape requires the response's single terminal
`yo.slice-review-result/v1alpha1` envelope. Gate preparation verifies its exact
review-chain ID, candidate, every requested lens, `clear` or `findings`
verdict, and the bidirectional consistency of finding IDs, summaries, and
affected lenses. It hashes the complete response as before but derives the
gate verdicts only from that closed envelope. It rejects a missing, duplicate,
non-terminal, stale, partial, or internally inconsistent envelope; it never
infers a verdict from surrounding prose. Stable v1 and v1alpha1 retain their
caller-declared `verdicts` behavior, while v1alpha2 rejects that field so the
two authorities cannot be mixed.

The experimental
`yo.slice-gate-prepare-request/v1alpha1` adds exactly one
`review_carry` value:

```json
{
  "schema": "yo.slice-gate-prepare-request/v1alpha1",
  "review_carry": {
    "schema": "yo.canonical-approval-review-carry/v1alpha1",
    "knowledge_id": "agent.example.unit"
  }
}
```

Use it only for the direct canonical approval follow-through described above;
all other fields remain those of v1. Stable v1 rejects `review_carry`, while
v1alpha1 requires it. Preparation verifies strict Git ancestry and the single
approval-path transition, asks Methexis to reproduce the exact record, derives
current-candidate review and approval identity, and returns
`yo.slice-gate-prepare-result/v1alpha1` with the carry proof. Every reviewed
validation command must declare `reused:true`; only v1alpha2 evidence with an
unchanged exact argv and an ancestor launch commit can satisfy it. This path
does not send another provider request, interpret new review prose, or bypass
the ordinary human approval requirement.

Preparation replays the complete original-or-delta review chain, requires the
validation command names to match every and only its final validation
artifacts, checks the current bound contract and clean candidate, validates the
generated gate request, and re-reads all inputs before atomically publishing a
new-or-byte-identical output. The same invocation returns the evaluated gate
result. New `yo.validation-run-summary/v1alpha2` evidence binds the launch
`HEAD`, clean worktree state, exact `argv` hash, complete-log hash, and the
`reviewed-descendant/v1` reuse policy. New opt-in
`yo.validation-run-summary/v1alpha3` evidence additionally records the target
OS and architecture plus a bounded Rust/Cargo toolchain fingerprint under
`reviewed-descendant-context/v1`. When a gate requests reuse, it observes that
context again and rejects changed platform or toolchain state automatically.
Its closed `external_state:"none-declared"` value means the producer asserted
that the command has no external dependency; commands that cannot make that
assertion must be rerun and must not use `--reusable-local`.

A gate entry with `"reused":false`
requires that launch HEAD to equal the candidate. A gate entry with
`"reused":true` accepts only a clean, passing summary whose exact command is
unchanged and whose launch HEAD trusted Git proves is an ancestor of the final
candidate. The summary itself remains `"reused":false` because it records an
execution; the gate entry records the later reuse disposition. Reuse is valid
only after the immutable review-delta chain includes and reviews that evidence
for the descendant candidate. Frozen `yo.external-operation-evidence/v1`
artifacts whose reviewed name starts with `external-operation/` also pass
through preparation without a copied summary. The gate revalidates their exact
candidate, embedded command arguments, expected and observed exit status,
counterfactual, and before/after observations against the prepared command.
They always require `reused: false`; a successful match is a passed
external-operation result and does not invent a validation log path. Frozen
`yo.validation-run-summary/v1`, `v1alpha1`, and `v1alpha2` evidence remains
accepted with its original meaning; v1alpha1 does not permit reuse. Legacy v1
does not record
launch arguments, so its coordinator-supplied exact `argv` remains a
declaration rather than launch proof. When exact human approval is
later recorded, add only its compact kind, authority, and scope to the
preparation input and publish to a new output path; the command supplies the
approved candidate and diff. It never executes a check, sends a review,
infers a verdict from response prose, creates approval, commits, or integrates.

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

Use `cargo xtask slice status <slice>` for the coordinator's ordinary progress
check instead of reopening raw Session JSONL, terminal panes, or all evidence.
It emits one `yo.slice-status/v1alpha2` JSON line containing clean `HEAD`,
review ancestry and round count, validation/gate artifact counts, delivery
claim/receipt counts, durable external request count, and one next action. Its
bounded scan reads at most 256 JSON files and never prints their content.
`build_review` means no applicable prior finding chain can be reused.
`review_delta` means the latest reviewed candidate is an ancestor of current
`HEAD` and its exact reviewer-authored finding set is present, so continue from
that manifest instead of rebuilding a full packet. `deliver_current_review`
means the current candidate already has a content-addressed packet and only its
authorized delivery remains. `prepare_gate` and `run_gate` likewise reuse the
existing current-candidate review and exact gate bytes rather than publishing
duplicates.

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

When that exact proposal covers all three accepted effects, record its gate
scope in this canonical form:

```text
yo.slice-accept-effects/v1alpha1;slice=<slice>;candidate=<commit>;squash=true;push=<remote>:<full-integration-ref>;close=true
```

Then `cargo xtask slice accept <request.json>` may perform ready-gate message
derivation, exact squash, accepted commit, non-force exact-ref push,
close-metrics preparation, close planning, and verified cleanup as one
orchestrated transition. The `yo.slice-accept-request/v1alpha1` request binds
the gate, message source, and close-preparation bytes by hash and names the
message output, close plan, remote, ref, and identical approval scope. It
revalidates both worktrees and all inputs immediately before the first
mutation and requires the staged and accepted diffs to equal the reviewed
candidate bytes. Before squash it evaluates commit impact against the
candidate paths and exact diff, so message or review-coverage errors leave the
integration worktree untouched. If the later pre-commit check or commit process
fails while the integration ref and exact staged bytes are still unchanged, it
restores those candidate paths to the original clean integration state. A
changed ref, non-exact index, conflict, accepted commit, or push failure remains
preserved for inspection. It never force pushes, invents approval, reruns
validation, or sends review.

```json
{
  "schema": "yo.slice-accept-request/v1alpha1",
  "slice": "<slice>",
  "gate_request_path": ".local-exclude/coordination/<slice>/gate.json",
  "gate_request_hash": "sha256:<gate-hash>",
  "message_source_path": ".local-exclude/coordination/<slice>/message.txt",
  "message_source_hash": "sha256:<message-hash>",
  "message_output_path": "/tmp/<slice>-commit-message",
  "close_prepare_request_path": ".local-exclude/coordination/<slice>/close-prepare.json",
  "close_prepare_request_hash": "sha256:<close-prepare-hash>",
  "close_plan_path": "/tmp/<slice>-close-plan.json",
  "push": {"remote": "origin", "reference": "refs/heads/develop"},
  "approval_scope": "yo.slice-accept-effects/v1alpha1;..."
}
```

Do not transcribe the ready gate identity, hashes, review trailers, validation
commands, approval scope, integration ref, or downstream artifact paths into
that request by hand. From the clean bound Slice worktree, put only the
irreducible semantic and observed inputs in one preparation request and run:

```bash
cargo xtask slice accept prepare \
  .local-exclude/coordination/<slice>/accept-prepare.json
```

```json
{
  "schema": "yo.slice-accept-prepare-request/v1alpha1",
  "gate_request_path": ".local-exclude/coordination/<slice>/gate.json",
  "message_source_path": ".local-exclude/coordination/<slice>/message.txt",
  "close_observations": {
    "execution_lanes": [
      {"lane": "integration", "mode": "serial", "operation_count": 1, "max_concurrency": 1}
    ],
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
    "elapsed_bottleneck": {"name": "review", "elapsed_milliseconds": 1000}
  },
  "push_remote": "origin"
}
```

The `yo.slice-accept-prepare-request/v1alpha1` input names the unchanged ready
gate, the human-written commit message source, the push remote, and
`close_observations`. Those observations are only facts the gate cannot derive:
execution-lane counts, review rounds and finding dispositions, packet sizes and
reuse, commands for known unverified environments, and the measured elapsed
bottleneck. Do not estimate or invent those values. The command verifies the
gate's existing exact-effect approval and both worktrees, validates the close
observations against the gate, then publishes new-or-byte-identical
`close-prepare.json` and `accept.json` in the Slice's standard coordination
directory. Candidate-scoped commit-message and close-plan output paths are
derived under the platform temporary directory.

The bounded result reports the base, candidate, diff, evidence/trailer counts,
approval scope, artifact paths, and hashes. Any changed gate or message bytes,
stale worktree/ref, approval mismatch, invalid observation, aliased path, or
conflicting prior output fails before either downstream request is published.
This preparation does not approve, integrate, push, or close the Slice. After
inspecting its result, use the generated exact request without rebuilding it:

```bash
cargo xtask slice accept \
  .local-exclude/coordination/<slice>/accept.json
```

If a required full suite fails outside the changed boundary, run each exact
failing test once in isolation to classify timing or shared-load sensitivity.
An isolated pass is diagnostic evidence, not a replacement for the failed
gate: rerun the original required suite and count both suite attempts and the
classification runs in the Slice close metrics. Do not loop retries or weaken
the assertion to manufacture a pass.

Before publishing immutable review input, finish the coordinator's actual-code
check and freeze the candidate, fixture prerequisites, planned Developer Docs
impact, review questions, and uniquely named validation evidence. A manually
assembled review runs review-packet preflight to expose the exact
managed-payload budget and section costs without publishing an eligible packet.
The integrated `review-prepare` path below performs the same input, scope, and
complete-packet budget checks in its single publication pass. Publish only when
no input is expected to change; preparation is not completed review.

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

The complete review, evidence, gate, integration, and Slice-cleanup workflow is
owned by [Review and integration](CONTRIBUTING/review-and-integration.md).
Read that authority before preparing or accepting a Slice review. This file
routes to it and does not duplicate its rules.
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
