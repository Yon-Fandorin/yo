---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.status.approval
revision: sha256:3aab7ccbfd550bf5f111fbf9d7a4bfe8ba5c03e61f52f0b2468fa812c734fc4f
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:d9ea262005dd159a39d645632d31769eded876e8af95bc6fc0073c0d246463e1
---
# Korean Review Projection

## Translation

# 파생 승인 상태

## 선언

승인 상태는 context eligibility와 분리된 파생 축이어야 합니다. 닫힌 label은 `draft`와 `approved`입니다. consumer는 이 축을 eligibility와 독립적으로 평가해야 하며, `approved`라는 사실만으로 revision이 일반 context에 들어갈 수 있다고 판단하면 안 됩니다.

`methexis.approval.exact-revision-binding`과 `methexis.approval.current-record`가 이 label을 파생하는 조건의 유일한 owner로 남습니다. 이 상태 정의는 해당 계약으로 routing해야 하며 exact-revision, proposal 또는 trusted-integration boundary를 다시 정의하거나 중복하거나 약화하면 안 됩니다.
