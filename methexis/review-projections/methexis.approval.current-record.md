---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.approval.current-record
revision: sha256:93f4782ba4c5226320440bb691cf2e22aaaff5fee6d3be52cdeaf69824ab8e76
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:9ac7db4995ef690cedd336af74369301e58b79fc7e8daa8062816cc09b5920f4
---
# Korean Review Projection

## Translation

# 현재 approval record

## 선언

각 `KnowledgeId`는 `methexis/approvals/` 아래에 최대 하나의 현재 approval record만 가져야 합니다. 현재 revision이 그 record와 다르면 unit은 Draft여야 합니다. 이전 approval record는 Git history에 남기며 Pilot은 historical revision마다 파일을 무한히 만들면 안 됩니다.

동일한 byte 쓰기는 idempotent해야 합니다. 다른 byte로 교체하려면 정확한 이전 `RevisionId`를 compare-and-swap 전제 조건으로 요구해야 하며 force 경로가 있으면 안 됩니다. working tree나 제안 branch의 일치하는 record는 검토를 위한 approval evidence일 뿐이며, 설정된 trusted integration commit에서 로드되기 전에는 effective approved 상태를 만들면 안 됩니다.
