# Workflow redesign: task-sized context and process

This is the rationale for the default workflow change, not an additional
policy checklist. The decision owner is [Contributing](../CONTRIBUTING.md).

## Comparison with mex

The upstream project is [mex-memory/mex](https://github.com/mex-memory/mex).
Its README describes a Markdown memory scaffold connected to implementation,
task-specific retrieval, and drift detection. Its
[agent template](https://github.com/mex-memory/mex/blob/main/templates/AGENTS.md)
reuses retrieved source and bounds unsuccessful graph exploration. Its
[router template](https://github.com/mex-memory/mex/blob/main/templates/ROUTER.md)
routes work to context and patterns, then updates knowledge after real work.
These are upstream design references, not measurements on yo.

| Concern | Previous yo practice | Adopted change |
|---|---|---|
| Start work | Every coding session requires a bound Slice | Ordinary checkout works immediately; bindings apply to formal work |
| Load context | Root workflow mixes policy, operation examples, and frozen schemas | Short default owner; operation references loaded on demand |
| Retain learning | Compact notes exist, but promotion can require a separate governance Slice | Update the existing owner within authorized scope; retain only reusable lessons |
| Validate | Complete workspace baseline at every Slice close plus selected hook checks | Affected behavior first; broad suites only for broad impact or formal gates |
| Review | Path-based lenses, exact packets, model classes, and trailers for normal code | Self-review by default; one independent review for meaningful risk; formal evidence opt-in |
| Commit | Blanket rejection of `-m`/`-F` and mandatory metadata | Ordinary Git messages; validate optional claims; strict explicit Slice commands |
| Finish | Gate, approval, metrics, close-plan, cleanup, and token retrospective | Result, checks, limits; formal closure only for formal Slices |
| Prevent context growth | Root had to exceed narrower owners by 2,000 tokens in a test | Absolute default-context token ceiling |

This adaptation is selective. mex's router also mandates architecture loading,
checklist narration, and post-task growth steps. Those would recreate fixed
costs here, so they are not adopted. Graph retrieval is an optional discovery
tool, not proof of completeness or a prerequisite to editing. We did not
install mex, run setup, enable telemetry, or add a second knowledge store.

## Boundaries and migration

Ordinary work no longer promises a machine-enforced independent review ledger
for every implementation commit. Risk-based review is an explicit contributor
responsibility. The ordinary hook validates supplied protocol claims; it does
not infer semantic risk or certify that a review happened. This is a deliberate
tradeoff that removes mandatory evidence construction from routine work.

Methexis accepted-design and activation authority, honest validation,
preservation of user changes, and actual authorization for external or
destructive effects remain. Existing bound Slices and immutable evidence chains
continue through their formal protocol. The existing strict CLI operations and
frozen schemas are retained; an ordinary commit is not a formal acceptance.

The formal procedure is compressed into `formal-slices.md`: lifecycle and
authority boundaries remain, while repeated schemas and command mechanics
are reached through executable owners and operation-specific references.
This removes over 1,000 lines of copied procedure from the first revision of
that reference. The three detailed packet/delivery/integration protocols remain
available for their specific operations; they are not a default reading set.

Hook selection is changed in `hk.pkl`; existing installations need `hk install`
to refresh hook events. Developer Docs describe both completion paths in English
and Korean. Tests cover ordinary commits, dishonest optional claims, formal
checks, affected authority routing, and an absolute startup token budget.

## Evidence and follow-up measurement

At the starting revision `bc24858d`, the default root was 1,168 lines / 8,036
whitespace-separated words. The four workflow owners together were 2,537 lines /
16,509 words. These are text-size observations, not model token or latency data.
The existing tokenizer test now caps the root at 1,300 tokens and the root plus
agent index at 1,700 using `o200k_base`; formal setup is capped at 1,800.
Do not increase an unrelated document
to make a relative saving assertion pass.

The revised root measures 90 lines / 658 words / 1,004 `o200k_base` tokens;
with `AGENTS.md`, the default workflow context is 1,341 tokens, down from 2,287
in the first revision. Formal setup is 109 lines / 1,268 tokens. These measure
the reading surface, not end-to-end agent token savings.

## Executable mex adaptations

The [upstream grounding and retrieval design](https://github.com/mex-memory/mex#grounded-in-the-code)
connects memory to code nodes, narrows retrieval, and surfaces changed anchors.
Here the lightweight implementation is `tools/context.py`, using only Python's
standard library and the existing Markdown owners:

- `find` ranks route/check table rows, prefers the requested outcome, and emits
  three anchored excerpts by default. It reports `no_match` and truncation;
  weak retrieval goes back to `rg` rather than triggering broad expansion.
- `impact --changed` includes staged, unstaged, deleted, and untracked paths
  and finds notes linking to them. This is a review hint, not semantic drift proof.
- `check` catches missing local file targets and repository escapes. It runs as
  a cheap hook when documentation or implementation paths change.
- Focused [recipes](recipes.md) connect recurring problems to their executable
  owner and the condition that invalidates the advice. No duplicate source
  snapshot, graph database, automatic rewrite, or daily log is introduced.

The helper does not implement mex's symbol fingerprints, call graph, semantic
rename rebinding, or multilingual retrieval. File-link checks cannot detect a
behavior change inside an existing file; impact hints and human inspection
cover that limit. All commands emit at most 12 KiB and make no network requests.

Workflow Markdown now runs the narrow context-budget test and link check,
not all xtask tests. `hk.pkl` or Rust implementation changes still run the
full affected xtask suite. Shared-workspace lint and runtime validation remain
unchanged; no passing checks or formal evidence were manufactured.

To assess actual throughput, sample ordinary changes and record only available
observations: context loaded, repeated check executions, review round trips,
elapsed time, and missed defects. Compare similar tasks before and after.
No per-task cost report is required, and no upstream benchmark percentage is
claimed as a measured saving for yo. Tighten the workflow only in response to
a concrete failure mode; do not restore blanket ceremony from hypothetical risk.
