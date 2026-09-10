# Formal Slice protocol

Use this only for a selected formal Slice, an existing bound Slice, a Wave,
or a Methexis authority/activation transition. Ordinary work follows
[Contributing](../CONTRIBUTING.md). Read the current operation's owner,
not every linked protocol. Existing CLI schemas and evidence remain strict.

## Work units and setup

A Slice is one independently reviewable outcome. A Wave groups outcomes with
a shared milestone or dependency gate; Task branches provide worker isolation.
Use `slice/direct/<outcome>` from `develop`, or `slice/<wave>/<outcome>`
from `wave/<wave>`. Add a Wave or Task only when coordination needs it.
Keep one semantic owner per decision and disjoint write leases. Create dependent
work from the accepted parent; do not freeze overlapping siblings at one base.

Keep transient contracts under `.local-exclude/coordination/`. A
`yo.slice-contract/v1` records the base/ref, owned contracts, closed write-set,
dependencies, focused checks, and close checks. Existing command examples and
schema validation live with
[Slice creation](../tools/xtask/src/slice_create/mod.rs) and
[contracts](../tools/xtask/src/slice_contract/mod.rs).
Declare narrow owning subtrees when cohesive work may add sibling files.

`cargo xtask slice create <contract.json>` discovers the clean integration
worktree, pins the base, checks active ownership, publishes the contract,
creates the branch/worktree, and binds it. Exact retries reuse completed
effects; preserve conflicting state. In a manually created Slice worktree use
`cargo xtask slice-contract bind <contract.json>` once.
Run `cargo xtask check slice-scope` on resume and before review. Reconcile an
out-of-scope dependency instead of silently widening the lease.
For concurrent work, use `check slice-parallel`; deferred shared composition
uses `check wave-assembly`. No fixed worker count or mandatory delegation.

## SOT-first changes

Read the active Checkpoint, affected Knowledge, and exact code anchors.
Resolve missing/conflicting accepted contracts before implementation.
Repository procedure remains owned by CONTRIBUTING; do not promote a new
procedure to canonical Knowledge without human acceptance of that transition.

Contract authoring, activation, and implementation remain separate Slices.
When Methexis advertises `canonical-approval-on-demand-projection/v1`:

1. Revise Source and canonical English Knowledge; check records and relations.
2. Commit the semantic candidate; complete the required independent lenses.
3. Resolve findings in child commits, reusing the reviewer for affected deltas.
4. Prepare canonical approval for the exact English revision. Generate a Korean
   Projection only for an explicitly requested Projection-basis approval.
5. Record actual human approval using the prepared basis; run
   `cargo xtask check methexis-check-for-stage`.
6. Integrate, activate through a separate Slice, verify trusted active authority,
   then implement against it.

Step 5 records the user's actual approval; it does not require asking again when
the conversation already authorizes the exact prepared basis. Follow the approval
retention rule in [Contributing](../CONTRIBUTING.md#decisions-and-memory). Carry
that authorization through the included integration and activation steps without
another permission question. Approval-record generation and mechanical metadata
changes do not themselves create a new semantic candidate requiring human review.
Do not claim that a general goal approves unseen semantic changes, or that a brief
approval supplies a separate review lens the user did not perform.

Without the capability, preserve the legacy Source/Knowledge/Korean Projection
candidate sequence. Existing approved revisions are not regenerated in bulk.
Approval-only review carry is allowed only when the existing CLI mechanically
proves the exact descendant and canonical approval bytes.

For activation use `cargo xtask slice create-activation <request.json>`
and follow its structured `next_actions`. Reuse prepare outputs and manifests.
Stage the returned transition paths before `methexis check --staged-activation`.
After integration, run ordinary full Methexis validation against trusted
`develop`. Setup, preparation, or a successful check never grants activation.

## Review and integration

Complete the declared baseline on the candidate; preserve dirty runs only as
diagnostics. Use the bounded runner for noisy evidence. Do not repeat unaffected
passing commands; formal reuse must satisfy the existing immutable chain/gate.
Preserve every published reviewed candidate as an ancestor; fixes are children.
Never amend or rebase a reviewed candidate to make evidence appear current.

| Operation | Entry point | Authority |
|---|---|---|
| Inspect current progress | `cargo xtask slice status <slice>` | Its exact `next_argv` or blocking reason |
| Publish original or delta review | `slice review-prepare` / `slice review-delta` | [Packets](review-packets.md) |
| Deliver or continue external review | `slice review-deliver` | [Delivery](review-delivery.md) |
| Consolidate validation/review/approval | `slice gate prepare` | [Integration](review-and-integration.md) |
| Accept a ready candidate | `slice accept prepare`, then returned request | [Integration](review-and-integration.md) |
| Close an already accepted Slice | `slice close prepare`, then `plan` / `apply` | [Integration](review-and-integration.md) |

All abbreviated commands above follow `cargo xtask`; inspect command usage
for arguments. Consume generated requests instead of transcribing hashes.
Preparation never grants approval or sends review. Respect existing external
route limits and request counts; retries do not imply renewed authorization.

The gate returns `validate`, `review`, `approve`, or `integrate`.
Finish earlier actions before presenting an exact approval proposal.
Reuse an existing human authorization when its scope and effects match.
A new material effect or candidate requires its appropriate disposition;
sandbox permission is a separate execution boundary.

Use `cargo xtask slice commit` for formal accepted commits; its strict
preflight and prepared message remain mandatory. Ordinary Git commits are
not formal acceptance. Preserve exact reviewed bytes through squash.
Do not claim a model review, human verdict, environment pass, or approval
that did not happen.

## Retention and cleanup

Use the compact derived `slice accept prepare` path when eligible. It derives
close records from existing evidence; do not reconstruct unobserved metrics.
Preserve unresolved material, then inspect the exact hash-bound close plan.
Apply only the authorized cleanup; keep accepted commits and unintegrated work.
For Waves, integrate Slices serially, reconcile contract conflicts with their
owners, and promote accepted commits to `develop` by fast-forward.
Keep frozen wire-format details in their executable owners and tests, not in
session-start prose.
