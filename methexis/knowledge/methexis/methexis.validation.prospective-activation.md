---
schema: methexis.knowledge/v1alpha1
id: methexis.validation.prospective-activation
kind: rule
owner: methexis
sources:
  - id: methexis.validation-model.prospective-activation
    revision: sha256:39dc7573028bde28c751733184caf9f557c47171b989dd6cf78a859722b83767
---
# Prospective staged activation validation

## Statement

`methexis check --staged-activation` is the repository-hook path for the otherwise unavoidable interval after revised approvals reach trusted `develop` and before their replacement Checkpoint is integrated. Without a staged active-record change it has exactly the ordinary all-class `check` behavior. With one, it accepts only one new immutable Checkpoint, the active record, and the complete registered tracked-artifact set in the Git index; unrelated staged paths fail closed.

The staged path is read-only and prospective, never trusted authority. It resolves `develop` once, reproduces the proposed Checkpoint from that exact trusted commit, verifies the active record's exact predecessor hash and canonical bytes, requires every selected Source to remain fresh, checks staged artifact provenance, and revalidates Source, proposal-index, and trusted-ref stability before returning. It pins the exact Git index selected by the commit invocation, including an explicit `GIT_INDEX_FILE`, and rejects non-regular or non-stage-zero entries. Success labels the candidate `prospective` and requires ordinary full `check` after the exact reviewed transition is integrated. It MUST NOT accept caller-selected refs, arbitrary future trees, working-tree-only candidate bytes, or a general hook exception.

One separate explicit review-only operation MAY consume an exact activation-request file inside a clean activation candidate worktree. It MUST resolve trusted `develop` once; capture that request, the one immutable proposed Checkpoint, and the canonical proposed active record; require the Checkpoint's stable authority basis to equal the pinned trusted commit; verify the active record's exact predecessor, approval closure, Source freshness, and complete registered manifest lineage; compile only the caller-named ContextBuild request; and final-revalidate every captured proposal file, Source observation, context request, and the trusted ref before returning or reusing an artifact. The result MUST use a distinct experimental schema, report `authority: prospective`, identify the exact trusted commit and proposed Checkpoint, and remain eligible only as input to the immutable activation-review packet procedure.

The review-only operation has a closed bootstrap. Its enabling Source and Knowledge revisions, implementation, versioned protocol family, and `CONTRIBUTING.md` workflow adoption MUST all pass the existing ordinary active-ContextBuild review and authority sequence before the operation is enabled. It MUST NOT build, verify, or supply evidence for a review, approval, activation, or workflow change that enables itself. The first eligible use is a later independent activation candidate after the enabling implementation and workflow are trusted and these contracts are active.

The review-only operation MUST NOT select a caller-provided ref, accept an arbitrary future tree, infer an activation proposal, fall back from failed ordinary context resolution, approve or activate the Checkpoint, satisfy general context eligibility, or make the candidate available to ordinary authority consumers. A prospectively compiled immutable ContextBuild MAY share its content identity with the same Checkpoint after activation, but ordinary reuse after integration MUST independently validate then-active trusted authority and current freshness.

This contract mechanizes the second half of a two-commit authority transition; it does not make revised approvals and their Checkpoint one authority commit. The trusted ref may therefore be intentionally inconsistent between the accepted approval commit and its exact back-to-back activation commit. During that bounded interval ordinary `check`, ordinary `resolve-context`, and all other authority-consuming operations continue to fail or use only valid active authority. Prospective review success never grants approval, activation, or context eligibility. The staged gate remains mandatory before integration, and ordinary full `check` remains mandatory after the exact transition reaches trusted `develop`.
