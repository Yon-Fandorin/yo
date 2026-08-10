---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.context.build-identity
revision: sha256:a3bb8f598446f350f1a46c5862bd0eb19a6a43707a33d5120513d823fa88e4e3
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:9fe0406cc4208555f40cb7ab55a6bfd29d2fe58c852e3a2aec6c0650b04c7eee
---
# Korean Review Projection

## Translation

Context resolution은 동일한 content-addressed 입력을 freshness 확인 뒤 재사용합니다. BuildId는 선택·관계·Source/evidence 관찰·candidate hash·compiler/tokenizer/budget 등 canonical plan의 domain-separated hash이며 경로·시각·현재 develop 관찰은 제외합니다. 초기 profile에는 model/permission filtering이 없습니다.

### 전체 정본 원문 대조

The user operation resolves a context; it does not rebuild one on every
request. Identical content-addressed inputs reuse an existing `BuildId` only
after the freshness guard passes. Relevant knowledge, relation, compiler,
projection, tokenizer, direct-anchor, exact candidate-input bytes, or budget
changes invalidate only affected results. The exact candidate-input hash is a
BuildId identity input; its physical input path is only a locator.

`BuildId` is the domain-separated SHA-256 of a versioned, length-delimited
canonical build plan. The plan contains the active-Checkpoint identity and hash,
its stable authority-basis commit, selected Knowledge revisions and required
relations, deterministic inclusion and omission decisions with their reason
codes, all Source and evidence observations that affected those decisions,
normalized direct anchors, the exact candidate-input hash, compiler and payload
profile, tokenizer profile, and maximum budget. It excludes the current
observation of `develop`, input and output paths, timestamps, result status, and
artifact hashes. Consequently an unrelated trusted-ref advance can reuse the
same build after final authority and freshness verification, while a change to
any relevant semantic input cannot.

The initial resolver request has no model or permission field and the first
profile performs no model- or permission-specific filtering. A future versioned
profile MAY add such inputs only together with their trusted derivation source,
selection semantics, and BuildId participation; a caller string alone cannot
grant content eligibility.
