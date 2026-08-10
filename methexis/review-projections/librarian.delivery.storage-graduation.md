---
schema: methexis.review-projection/v1alpha1
knowledge_id: librarian.delivery.storage-graduation
revision: sha256:8d250a2266020d7150a2d09b88106db21313de99c0615f9e21030fbaf44b6660
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:1ebdef2cdb9b4b1e8c5e1a23ec443ed34d4c2a8fd62bd4a040723be00eb32ee5
---
# Korean Review Projection

## Translation

Pilot Librarian은 요청마다 메모리 카탈로그를 재구축하고 데이터베이스나 상시 서비스를 도입하지 않습니다. 독립 저장소로 졸업할 때 계약 fixture를 먼저 옮기며 yo에는 얇은 adapter만 남기고 병렬 구현을 유지하지 않습니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

The Pilot rebuilds one immutable in-memory catalog from the captured
working-tree files for each request. It does not introduce a database,
persistent index, storage trait, or background service before corpus evidence
justifies one. Any later index remains reconstructible and non-authoritative.

The initial Librarian implementation incubates under `yo/tools/librarian`.
Validated capabilities and contract tests later graduate to a standalone
Librarian repository after Surface and SOT operating-procedure dogfooding.
Contract fixtures transfer before implementation, `yo` retains a thin adapter,
reference corpus, contract fixtures, and integration evaluation, and the two
repositories MUST NOT maintain parallel implementations. The destination
repository and reconciliation with any existing Librarian code are decided from
that evidence; the Pilot directory is not copied wholesale.
