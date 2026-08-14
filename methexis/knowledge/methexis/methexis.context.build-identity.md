---
schema: methexis.knowledge/v1alpha1
id: methexis.context.build-identity
kind: rule
owner: methexis
sources:
  - id: methexis.context-model.build-identity
    revision: sha256:1b5e549989439fd3ad09e70f7f01a1377053a69988be6fc2b65e5b8c5e63d81c
relations:
  depends_on:
    - methexis.context.bundle-packing
    - methexis.context.source-freshness
---
# ContextBuild identity and invalidation

## Statement

The user operation resolves a context; it does not rebuild one on every request. Identical content-addressed inputs reuse an existing `BuildId` only after the authority-mode-specific freshness guard passes. Relevant knowledge, relation, compiler, projection, tokenizer, direct-anchor, exact candidate-input bytes, or budget changes invalidate only affected results. The exact candidate-input hash is a BuildId identity input; its physical input path is only a locator.

`BuildId` is the domain-separated SHA-256 of a versioned, length-delimited canonical build plan. The plan contains the exact context Checkpoint identity and hash, its stable authority-basis commit, selected Knowledge revisions and required relations, deterministic inclusion and omission decisions with their reason codes, all Source and evidence observations that affected those decisions, normalized direct anchors, the exact candidate-input hash, compiler and payload profile, tokenizer profile, and maximum budget. It excludes the current observation of `develop`, input and output paths, timestamps, result status, artifact hashes, and whether the exact Checkpoint was observed as trusted active authority or through the explicit activation-review-only prospective guard.

Trusted-active resolution and activation-review-only prospective resolution of the same exact Checkpoint therefore derive the same BuildId because they compile the same semantic payload. The structured operation result and every consuming review plan MUST record the authority mode outside BuildId and apply the matching final guard. Before activation, a prospective artifact is eligible only for the immutable activation-review packet that captured its exact activation request and lineage. After activation, ordinary resolution MAY reuse those immutable bytes only after it independently proves that the same Checkpoint is then active trusted authority, current Source freshness still holds, and normal managed-build verification succeeds. Prospective success alone never makes a build generally eligible.

Consequently an unrelated trusted-ref advance can reuse the same build after final authority and freshness verification, while a change to any relevant semantic input cannot. Changing only the observation from prospective review to trusted active does not duplicate an identical build, but it always reruns the authority-mode-specific guard.

The initial resolver request has no model or permission field and the first profile performs no model- or permission-specific filtering. A future versioned profile MAY add such inputs only together with their trusted derivation source, selection semantics, and BuildId participation; a caller string alone cannot grant content eligibility.
