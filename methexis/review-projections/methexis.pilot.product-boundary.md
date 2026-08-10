---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.pilot.product-boundary
revision: sha256:6c901e0d915d8f2b9119ba7d10baec0bbce4ababeb27a490168f2ad0c86b3e52
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:c6410ca97b334e273aa9a64f8dc8e75d1038fc752f63024298fb78e91f0ae79a
---
# Korean Review Projection

## Translation

Methexis는 먼저 yo 내부 Pilot로 시작하며, yo는 검증용 첫 소비자이지 영구 소유자가 아닙니다. workflow 정본은 CONTRIBUTING.md에 남고, 도메인 일반화는 두 번째 실제 제품 소비자의 증거가 생기기 전까지 금지됩니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

`tools/methexis` MUST begin as an internal `yo` Pilot. Its first job is to
improve code-agent work on `yo`; it is not yet a generic knowledge platform.

`yo` is the incubation testbed and first reference consumer, not the expected
permanent owner of Methexis. Repository extraction and domain generalization
are separate gates: validated Pilot capabilities MAY move to a standalone
Methexis repository, while generalizing beyond the `yo`-proven contract
requires evidence from a second real product consumer.

A small SOT operating-procedure corpus MUST provide a structurally different
secondary sample. It MAY reference the repository workflow authority but MUST
NOT restate or become a second canonical owner for `CONTRIBUTING.md` policy.
Existing workflow rules remain references or generated projections; new
KnowledgeUnits own only SOT-specific procedures not already owned elsewhere.

The domain model MUST NOT contain TUI-specific fields. General domain expansion
beyond the `yo`-proven contract requires a second real non-`yo` product
consumer.
