---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.validation.working-tree-authority
revision: sha256:868a1458ac2524450930e8a252072309e32fb7e6976d68fe5da869a345be79e6
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:9b57fb31b7547b445f80b43567beb8226d9edb813d6d83bdbdd32a35f73299fe
---
# Korean Review Projection

## Translation

`methexis.validation.snapshot-construction`이 소유한 구조 record 검증이 성공한 뒤, working-tree Fast Check는 현재 Draft Knowledge와 typed Source를 기준으로 한국어 review Projection과 approval proposal을 평가해야 합니다. proposal evidence를 `matching_proposal`, `stale_proposal`, missing으로 보고할 수 있지만 로컬 evidence가 trusted approval이나 activation을 부여하면 안 됩니다. 이 unit은 구조 record 검증을 다시 정의하면 안 됩니다.

Fast Check는 `methexis.status.approval`이 파생한 approval axis와 `methexis.status.eligibility`가 파생한 최종 eligibility를 소비해야 하며 어느 상태도 다시 정의하면 안 됩니다. 현재 working tree 또는 host observation은 해당 status 계약이 연결하는 demotion guard를 통해서만 기여할 수 있고 Draft, inactive, unapproved content를 승격하면 안 됩니다.

성공 보고서는 trusted integration에서 파생된 상태를 포함하더라도 보고서 자체의 authority를 Draft로 식별해야 합니다. trusted status 평가가 실패하면 Fast Check도 실패를 반환해야 하며 로컬 proposal evidence를 trusted state로 대체하면 안 됩니다.
