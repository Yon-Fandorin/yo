---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.migration.reversal-transition
revision: sha256:b2825032abe4453e716744ae2c3f127ab09dadb1b8e1291a383fe0ce64819667
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:9c154c217a1dc4a35c41f46909fc88adaf52d3f7585902f10d8810a2a72bd728
---
# Korean Review Projection

## Translation

Migration 이후 owner를 되돌리거나 교체하려면 scope-preservation의 reviewed revision 또는 successor가 새 owner 집합을 명시해야 합니다. 현재 owner는 새 owner가 forward CAS activation으로 trusted될 때까지 권위를 유지하며, raw Git revert·삭제·pre-migration Checkpoint·working-tree proposal은 권위 전환이 아닙니다.

### 전체 정본 원문 대조

Reversing or replacing any post-migration SOT authority assignment requires an explicit reviewed revision of `methexis.migration.scope-preservation`, or an explicit semantic successor, that names the replacement owner set. A new forward compare-and-swap Checkpoint activation MUST keep every current owner authoritative until its exact replacement becomes trusted. A raw Git revert, deletion, pre-migration Checkpoint, working-tree proposal, or caller-selected ref is not an authority transition and MUST NOT revive the historical document prose.
