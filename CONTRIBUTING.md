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

Read the relevant AGENTS routes once. Inspect the branch and
`git status --short --untracked-files=all`; preserve existing work.
Use exact paths or `rg`. For unfamiliar work,
`python3 tools/context.py find "prompt cursor"` returns a few existing
documentation routes. If they do not answer the question, use `rg`; rephrase
at most once. Retrieved source counts as read. Stop discovery when the owner,
affected behavior, and useful check are understood.

Reuse the checkout unless isolation helps. An optional branch is
`change/<outcome>`; `develop` is the usual integration target, `main` the
release boundary. Do not fetch, switch dirty work, or rewrite history for ceremony.

## Decisions and memory

Resolve implementation details and review fixes within the authorized outcome.
Ask once for a missing product/contract choice with concrete effects; preserve
earlier authorization across continuations. Authorization for external effects,
push, or destructive recovery comes from the user, not workflow prose.

Before asking, check the conversation and handoff. A short affirmative reply
approves the concrete proposal's scope, including its stated integration and
activation. Compaction, continuation, a worktree or approval record does not
expire authorization. Replace stale approval-wait notes with the approved scope
and next action. Ask again only for a material effect outside that scope or an
actual revocation, and identify the delta.

Goal tracking is not permission. Continue approved work without recreating the
goal, marking unfinished work complete, or restarting approval on continuation.
If an additional effect needs approval, finish independent authorized work and
make that effect concrete before asking only about the delta.

Code and tests own actual behavior; Methexis owns accepted design; Developer
Docs own navigation and checks; this page owns work practices. Update the
existing owner instead of copying its facts. Read the active Checkpoint and
affected Knowledge only when the contract matters.

Keep a short local handoff only for unfinished multi-session work: outcome,
paths, decisions, checks, unresolved work, next action. Retain a reusable lesson
only if it prevents rediscovery, with a code/command anchor and a condition that
would invalidate it. No daily log, transcript, mandatory retrospective, or
separate governance task for an in-scope lesson.

When parallel work is authorized, assign a cohesive batch, owned files and
contract pointers to each agent. Agree on interfaces before editing. Collect
batches before shared compilation; serialize shared checks and review the stable
patch once. Avoid tiny followups, unrelated reassignments and repeated source
or packet dumps. If coordination dominates, reduce concurrency and finish the
current batch; authorization remains valid.
`python3 tools/context.py impact --changed` finds documentation references to
changed files; `check` detects broken local references. These are navigation
hints, not semantic freshness or contract-approval proofs.

## Validation

Use the [affected boundary](docs/src/validation/README.md#start-from-the-changed-boundary).
Start focused; finish with affected package/consumer checks and `git diff --check`.
Use full workspace suites for shared runtime/build impact, releases, or unresolved
cross-package effects. Keep platform gaps visible. Reuse passing checks while
their relevant inputs, command, and environment are unchanged.
For noisy commands use `tools/validation/bounded-run.sh`; inspect logs on failure.
Do not duplicate a suite merely for review, commit, or cleanup.

## Review and integration

Self-review the actual diff, including new files. Obtain one independent review
before integrating public-contract, permission/security, concurrency, failure,
workflow-authority, or semantic-SOT changes. Routine approved implementation and
mechanical edits need no mandatory second reviewer. Review can be a patch plus
brief context; packet, model tier, and structured verdict are optional.
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
