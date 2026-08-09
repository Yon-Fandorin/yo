---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.knowledge.body-contract
revision: sha256:2c8c44d2ab0991e1bd67b1e763976863d32bf9c40fb15b1daf6fc08ad834292e
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:8a1b48333d73b2a74f24c509b2b3566a513b47bb3381c3f8c164fdc850caac41
---
# Korean Review Projection

## Translation

# Canonical 본문 계약

## 선언

모든 canonical KU 본문에는 비어 있지 않은 `Statement` 절이 있어야 합니다. Decision에는 비어 있지 않은 `Rationale` 절도 있어야 합니다. Procedure에는 비어 있지 않은 `Steps`와 `Completion Criteria` 절도 있어야 합니다.

Canonical 본문에는 raw HTML block이나 HTML comment가 있으면 안 됩니다. Code fence 안의 내용이나 rendering에서 숨겨진 내용은 필수 semantic section을 충족할 수 없습니다.
