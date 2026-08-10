---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.context.candidate-input
revision: sha256:e147dc178b577efadfb37290afce95af239e5a7d3c29a0e85bcfc20ea1f926da
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:1f0f87e6f26f36e96f3eb4eefbfa5f30ca36193fb8cc9423c609f0ac3003cc14
---
# Korean Review Projection

## Translation

candidate 파일은 repository root 아래에서 symlink 없이 bounded immutable snapshot으로 캡처됩니다. Methexis는 Librarian 검색을 재구현하지 않고 닫힌 wire contract와 정렬·hash·score 일관성만 독립 검증합니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

A candidate path must remain beneath the opened repository root. Capture
rejects absolute paths, empty or dot components, `..`, symlinks, non-regular
files, and files over the compiler profile's declared bound. It opens path
components relative to retained directory handles, captures one bounded byte
snapshot, and verifies file identity before and after capture. A concurrent
change is a structured retryable failure with no partial result or automatic
retry.

Methexis validates the candidate wire contract rather than reimplementing
Librarian retrieval. Its independent closed decoder validates every envelope,
identity, compiler, candidate, path, reason, unresolved-anchor, and truncation
field defined by the versioned candidate-set schema. It rejects unknown fields,
duplicate candidates or reasons, collection ordering that the schema declares
canonical, malformed or inconsistent hashes and candidate-set identity, a false
success marker, a candidate score unequal to the sum of its reason scores, and
candidate ordering that is not descending score then ascending KnowledgeId.
Cross-tool golden fixtures pin the complete accepted and rejected wire shapes.
Methexis does not recompute reason signals or fixed score weights, candidate
recall, or whether Librarian found the best result; reason scores determine
advisory order, not authority or eligibility.
