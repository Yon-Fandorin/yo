---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.status.demotion-evidence
revision: sha256:3ffdeca5953ebe40b36cab76f45a0a9dd233376851a7ca5f74900a1c7d9ea6a9
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:d961529b3203a89f91d949d06b874659030c9f6bf08d7bd7160ed858f0a909b7
---
# Korean Review Projection

## Translation

# 사전 전환 및 런타임 상태 demotion evidence

## 선언

사전 전환 및 런타임 status guard는 결정론적인 schema, graph, integrity, Checkpoint 실패와 명시적인 human invalidation을 `invalid`로, 해결되지 않은 review hold를 `suspect`로, 고정된 Source, evidence result, retrieval 또는 attestation이 Knowledge revision에 대해 승인된 freshness input을 더 이상 만족하지 않는 경우를 `stale`로 판정해야 합니다. ineligible winning condition 순서는 `invalid > suspect > stale`여야 합니다. durable negative input은 `methexis.status.negative-record`가 소유합니다.

현재 working-tree나 host observation은 guard outcome을 낮출 수만 있고 approval이나 activation을 부여하면 안 됩니다. 모든 guard outcome은 우선순위와 전환을 테스트할 수 있도록 winning condition의 machine-readable evidence를 포함해야 합니다.

고정된 Source가 바뀐 뒤 시작한 resolution은 영향을 받는 knowledge와 Projection을 차단하고 영향을 받는 모든 Checkpoint를 degraded로 표시해야 합니다. winning ineligible 상태는 required graph에서 영향을 받은 unit을 transitively 필요로 하는 선택된 dependent 방향으로만 전파되어야 합니다. 영향받지 않은 prerequisite, sibling, 관련 없는 approved knowledge는 eligible 상태를 유지해야 합니다. resolution 도중 Source가 바뀌면 서로 다른 observation을 섞어 조용히 허용하거나 재시도하지 말고 `SOT-007`이 소유한 immutable snapshot과 final revalidation 규칙을 따라야 합니다. 이 unit은 동시 변경 사례를 `SOT-007`로 routing할 뿐, 해당 규칙을 복사하거나 소유하지 않습니다.
