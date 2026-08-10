---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.migration.complete-transition
revision: sha256:2bffdaf312ff356aa3a8fc459ecfcd92eb1bc9c3086c3d4aa9aadb3a2050cbdd
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:a3fe11556bf2fadb3ab324e000ea43a5cd64314a951af208697f53857ed66087
---
# Korean Review Projection

## Translation

완전 이관 전에는 이미 active KU에 위임된 scope를 제외하고 SOT Pilot 문서가 권위를 유지합니다. 이관은 complete-transition, scope-preservation, reversal-transition와 전체 owner closure를 한 번의 forward CAS Checkpoint로 함께 선택해야 하며 partial selection은 아무 scope도 넘기지 않습니다. trusted 이후 KU들이 sole authority가 되고 문서는 routing Projection이 됩니다.

### 전체 정본 원문 대조

Before the complete migration becomes trusted, `docs-internal/design/sot-pilot.md` remains the sole authority for every scope not already delegated to an active semantic KnowledgeUnit. Existing active KnowledgeUnit revisions remain authoritative for their already delegated scopes.

The complete migration MUST be one forward compare-and-swap Checkpoint transition that selects an exact approved revision of `methexis.migration.complete-transition` and its complete required closure. That closure MUST include `methexis.migration.scope-preservation`, `methexis.migration.reversal-transition`, and every exact scope owner required by `methexis.migration.scope-preservation`. Partial selection transfers no remaining document-owned scope.

Once that exact transition becomes trusted, the scope-owner KnowledgeUnits become the sole authority for their assigned scopes and `docs-internal/design/sot-pilot.md` becomes a non-authoritative routing Projection. The currently authoritative revisions remain authoritative until the exact replacement transition becomes trusted; replacement revisions need not already be active.
