---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.context.source-freshness
revision: sha256:88d268a12be4cb847fc49b09513b15442d2412483994db8e62196df2ce611f25
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:61e8af7a28219e0087a4e1aaeed33f029ceac21b21389ec8addc18274f84d2c7
---
# Korean Review Projection

## Translation

freshness guard는 cache hit를 포함해 매 resolution마다 실행됩니다. code/external Source를 fail-closed 방식으로 검증하고, publish 직전 mutable Source와 trusted authority identity를 다시 확인해 동시 변경이면 아무것도 publish하지 않고 retryable failure를 반환합니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

The freshness guard runs on every resolution, including a cache hit. It compares
the trusted commit and active-Checkpoint hash, referenced KnowledgeUnit hashes,
approval revisions, and required evidence hashes. For a code Source it resolves
the recorded locator against the current working tree, captures the bytes and
file identity, and hashes that immutable snapshot. A missing locator, dirty
change, or hash mismatch is drift rather than an implicit authority revision.

The code guard hashes exact whole-file bytes in v1; it does not normalize line
endings or extract a symbol range. It walks repository-relative path components
without following symlinks, retains the opened file while capturing bytes,
checks identity before and after capture, and reopens and rehashes immediately
before returning. A stable missing file or hash mismatch is stale, a path escape
or symlink is invalid, and a concurrent identity or byte change returns the
retryable `source_changed_during_validation` failure without a partial result or
automatic retry.

External Sources use one enforceable freshness mode:

- immutable or versioned: verify the pinned identifier and captured hash;
- mutable and retrievable: retrieve current content and compare its hash;
- opaque or unavailable: require a human attestation with a fixed expiry.

Missing retrieval, missing attestation, or expired attestation fails closed.
The Pilot need not implement a generic external connector until its corpus
requires one. The guard does not rerun executable validation.

The resolver compiles only from its captured Source snapshot. Immediately before
publishing a new artifact or returning a cached one, it rechecks every observed
mutable Source identity and hash and compares the current trusted-ref and active
Checkpoint identities with the values captured at operation start. A concurrent
mismatch publishes nothing and returns a structured retryable
`source_changed_during_resolution` or `authority_changed_during_resolution`
failure.
This whole-operation failure also applies when the concurrently changed Source
belongs only to an optional candidate: the resolver cannot claim a consistent
snapshot assembled partly before and partly after that change. It does not
retry automatically.
