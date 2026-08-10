---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.migration.reversal-transition
revision: sha256:9213ea5464d840c6e9783c990166b6e6d1da986594af92d37cdb4b34d4c6711c
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:b905ace1e90bbf21b8f33ef209c95a42f086a428e3922472b45b38586ac99840
---
# Korean Review Projection

## Translation

SOT owner를 교체하려면 scope-preservation의 reviewed revision 또는 명시적 successor가 새 owner 집합을 지정해야 합니다. 새 owner가 forward CAS Checkpoint로 trusted될 때까지 현재 owner가 권위를 유지합니다. Git revert, 삭제, 이전 Checkpoint, working-tree proposal, caller-selected ref는 권위 전환도 대체 권위도 아닙니다.
