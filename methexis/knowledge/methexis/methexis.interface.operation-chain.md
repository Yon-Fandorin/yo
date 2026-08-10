---
schema: methexis.knowledge/v1alpha1
id: methexis.interface.operation-chain
kind: rule
owner: methexis
sources:
  - id: methexis.interface-model.operation-chain
    revision: sha256:00b7a9a5e16b38bc26a1fdc21058313b4fe4f4a43ca882a1648c1c80146f9eb4
---
# Methexis operation chain and authority boundaries

## Statement

The implemented operations are:

```text
author-revision <request.json>   -> derived revision authoring Draft proposals
project-review <request.json>  -> tracked Korean review Projection
build-review <request.json>    -> local packet and manifest
prepare-approval <manifest.json> --reviewer <owner-id> [--replace-current]
                               -> approval-request proposal on stdout only
approve <request.json>         -> tracked exact-revision approval proposal
prepare-checkpoint             -> Checkpoint-request proposal on stdout only
create-checkpoint <request.json> -> immutable trusted-revision Checkpoint proposal
prepare-activation <create-output.json>
                               -> activation-request proposal on stdout only
propose-activation <request.json> -> active-record proposal with compare-and-swap
check [--only <class>[,<class>...]]... [--summary] [--unit <knowledge-id>]
                                -> selected SOT integrity classes and their prerequisites
check --staged-activation       -> ordinary check or one exact staged prospective transition
resolve-context <request.json>  -> immutable ContextBuild locator and hashes
```

`author-revision` collapses the revision-authoring loop into one call: it
accepts new Source content, a new Knowledge body, and/or new Korean review
Markdown, then derives the SourceRevision, the Knowledge source pin and
RevisionId, the replacement Projection, and the review packet, writing the
tracked files as Draft proposals. The unit's single decision Source id and all
other Knowledge metadata are preserved. Approval records MUST NOT be written
by this operation; human approval remains a separate explicit step. Units
that do not pin exactly one `decision` Source fail closed. Writes are
sequential per-file compare-and-swap operations rather than one batch; a
mid-sequence failure names the paths already written, and re-running the same
request converges the remainder.

The `prepare-approval`, `prepare-checkpoint`, and `prepare-activation`
operations remove hand-copied hashes from the review→approval→checkpoint→
activation loop. Each reads values that already exist in the repository — the
review packet manifest, the active Checkpoint roots, or one saved
`create-checkpoint` result — binds them into the exact request wire shape the
next operation consumes, and prints that request JSON on stdout. The authority
boundaries are unchanged: the prepare operations emit proposals only and never
perform the following mutation. `prepare-approval` MUST NOT write
`methexis/approvals/` or record an approval; human authorization remains the
separate explicit `approve` step. Checkpoint and activation preparation MUST
NOT invoke Checkpoint creation or activation. Missing authority inputs — an
unknown reviewer, a `--replace-current` without an existing approval record,
or no active Checkpoint — fail closed with structured diagnostics.

S4 adds context resolution with a versioned request and one structured result.
Success returns only the small artifact locator and integrity record described
by `SOT-007`; the completed context is not streamed implicitly. Failures
distinguish stable required-input failures from retryable concurrent Source or
authority changes. Stable ineligibility or unaffordability of an optional
candidate remains a successful build with an omission record; malformed input
or an integrity failure still fails the operation. Neither direct anchors nor
candidate input may override the trusted commit, active Checkpoint, approval,
or freshness guards.
