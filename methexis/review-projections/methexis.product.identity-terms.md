---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.product.identity-terms
revision: sha256:9535c62d48cf5dfa112555401eb5075231ec94b26d47ab11d7624f21b058c5e5
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:8747b66973e1518adaeab8a261e300f3197ff21da7164dafcce3a8e92cc89180
---
# Korean Review Projection

## Translation

Methexis는 승인된 canonical knowledge와 Projection 사이의 합의를 유지하는 제품입니다. SOT는 제품명이 아니라 architecture authority role과 stable decision ID prefix입니다. MUST/MUST NOT은 blocking contract이고 SHOULD 위반에는 이유가 필요하며, 예시 경로·명령·field는 고정 public API가 아닙니다. SOT decision ID는 교체 이후에도 안정적으로 유지됩니다.

### 전체 정본 원문 대조

Methexis is the product that maintains agreement between approved canonical
knowledge and its Projections. SOT names the architectural authority role and
remains the prefix for stable decision IDs; it is not the product name.

`MUST` and `MUST NOT` are blocking contracts. `SHOULD` requires a documented
reason to deviate. Illustrative paths, commands, and field names are not frozen
public API.

IDs remain stable even if a decision is later replaced. Downstream design,
Slice contracts, tests, and evidence reference these IDs instead of copying
their rules.
