# Contributing to yo

Make the requested change, verify its affected behavior, report the result.
This page selects the workflow; detailed protocol references do not expand an
ordinary task. No branch, plan file, Slice contract, packet, or metrics required.

For user-facing features, compare the actual interaction with representative
commercial products and the user's named references before choosing the flow.
Use the user's operating environment, including SSH and tmux, in that comparison.
Verify the complete user journey; a backend capability or fallback command alone
does not establish equivalent usability. Reuse relevant current comparisons.

## Starting work

Read relevant AGENTS routes once; inspect the branch and
`git status --short --untracked-files=all`. Preserve existing work.
Use exact paths or `rg`. For unfamiliar work,
`python3 tools/context.py find "prompt cursor"` returns documentation routes.
If unhelpful, use `rg` after at most one rephrase. Stop once the owner,
behavior and useful check are understood.

Reuse the checkout unless isolation helps. An optional branch is
`change/<outcome>`; `develop` is the usual integration target, `main` the
release boundary. Do not fetch, switch dirty work, or rewrite history for ceremony.

## Decisions and memory

A task request approves necessary implementation, fixes, reviews, transfers and
checks on configured, authenticated targets within the user's constraints.
Routine layout choices within accepted behavior and ownership are included.
For Codewright's Confirm shape, show the tree, owners, dependencies and shared
surface before editing, then continue. Wait for a missing product/owner decision
or explicitly requested design sign-off.
Approval survives corrected candidates, new hashes, compaction, continuations
and worktrees; record its task origin. Check conversation and handoff before
asking. A short affirmative approves the concrete proposal, including stated
integration and activation. Replace stale approval-wait notes with scope and
next action. Ask only for missing decisions, revocation or effects outside scope;
identify the delta. Push and destructive recovery require their own authorization.

Execution-environment approval checks remain controlling. Report rejected
actions with reasons; never bypass checks.

Goal tracking neither grants permission nor expires approval. Do not mark
unfinished work complete. Pending approval blocks only dependent actions;
continue independent authorized work and ask about the delta.
Bound formal Slices retain their exact approval and dependency gates.

Code/tests own behavior; Methexis owns accepted design; Developer Docs own
navigation/checks; this page owns work practices. Update owners, not copies.
Read the active Checkpoint and affected Knowledge only when contracts matter.

For unfinished multi-session work, keep a local handoff: outcome, paths,
decisions, checks, unresolved work, next action. Retain lessons that prevent
rediscovery, with code/command anchors and invalidation conditions. No daily
log, transcript, mandatory retrospective or separate in-scope governance task.

When agents are authorized by the task or applicable instructions, assign
independent batches, owned files, contract pointers and agreed interfaces.
No separate confirmation per agent, worktree, review or corrected candidate.
Serialize conflicting writes and checks sharing mutable inputs or outputs;
isolated checks can overlap. Collect dependencies before integration and review
the stable patch. Avoid tiny followups and repeated source or packet dumps.
Reduce concurrency when coordination dominates; authorization remains valid.
`python3 tools/context.py impact --changed` finds documentation references to
changed files; `check` detects broken local references. These are navigation
hints, not semantic freshness or contract-approval proofs.

## Validation

Use the [affected boundary](docs/src/validation/README.md#start-from-the-changed-boundary).
Start focused; finish with affected package/consumer checks and `git diff --check`.
Use [full workspace suites](CONTRIBUTING/validation-execution.md) for shared
runtime/build impact, releases, or unresolved cross-package effects. Reuse
passing checks with unchanged inputs, command, and environment.
For noisy commands use `tools/validation/bounded-run.sh`; inspect logs on failure.
Do not duplicate a suite merely for review, commit, or cleanup.

## Review and integration

Self-review the actual diff, including new files. Obtain one independent review
before integrating public-contract, permission/security, concurrency, failure,
workflow-authority, or semantic-SOT changes. Routine approved implementation and
mechanical edits need no mandatory second reviewer. Use a patch and brief context;
match model capability to risk. Packets and structured verdicts are optional.
Do not launch agents or transmit data without authorization. If review is
unavailable, finish and validate the change, then report the remaining review.

Commit when included in the request; ordinary `git commit -m`/`-F` work.
Trailers are optional; supplied review/docs claims are checked.
Do not invent evidence or repeat approval for already authorized local steps.
Report the outcome, checks, and remaining limits; push/integration/cleanup stay
within the authorized effects. Never rewrite shared history or force-push
without explicit approval. External source trees, including `rib`, are read-only.

## Slice Contract

Use [formal Slices](CONTRIBUTING/formal-slices.md) only when explicitly selected,
continuing a bound Slice, coordinating a Wave, or changing Methexis authority.
Existing bindings and published review lineage remain binding; do not abandon
them to bypass a gate. A missing Slice binding on an ordinary checkout is normal.
Keep one owner per mutable boundary; serialize shared writes.

## Test code

Test an observable contract and a plausible failure, not an implementation copy.
Keep Korean explanations above built-in Rust `#[test]` attributes. Assert the
value actually consumed at handoffs and the first excess unit at capacity limits.
Bound blocking I/O and process waits; clean up owned resources.

## Local checks

`hk.pkl` selects hooks; `cargo xtask` implements structured checks.
Install pinned tools when missing (`hk` 1.52.0, `mdbook` 0.5.4); run
`hk install` after hook event changes. `hk check` checks; `hk fix` can edit.
The ordinary hook is `change-preflight`; `commit-preflight` remains formal.
The [mex comparison](CONTRIBUTING/workflow-redesign.md) is background, not a checklist.
