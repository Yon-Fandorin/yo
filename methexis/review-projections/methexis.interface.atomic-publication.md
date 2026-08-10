---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.interface.atomic-publication
revision: sha256:7eb55191c68ba58115f6427434505541bae272180b84c7d4066db4600c488b07
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:509223053d6c0a5c818291497a3de5846750879e442e3f53ceabf94f69775c0c
---
# Korean Review Projection

## Translation

모든 mutation은 symlink 경로 전환을 막고 target별로 writer를 직렬화하며 atomic하게 publish합니다. 변경에는 정확한 이전 hash/revision CAS가 필요하고 실패 시 기존 record를 유지하며 partial output을 노출하지 않습니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

Every mutation publishes atomically and rejects symlinked output parents.
Publication resolves and retains directory handles before locking or writing;
a concurrent parent rename or symlink swap cannot redirect output outside that
opened repository directory.
Tracked mutations serialize concurrent writers per target. A different
Projection requires its exact prior content hash; a different approval requires
its exact prior RevisionId. Checkpoints are immutable; active-record replacement
requires the exact prior record hash. Failures leave the prior record unchanged
and expose no eligible partial output.
