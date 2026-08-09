---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.source.reference-pinning
revision: sha256:49db80fa7bb0cb5de027e8ce5d7995568a83ec6da743ef24a9b61b4ee1e697ae
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:06f73015cf659605a6b289a31f17e50508f2e8e5706a1c84b6afd2a7228fb4e9
---
# Korean Review Projection

## Translation

# 정확한 Source reference 고정

## 선언

KnowledgeUnit은 각 Source를 정확한 typed pair인 `{SourceId, SourceRevision}`로 고정해야 합니다. Source record는 자신의 location, content 또는 external reference, revision을 한 번 소유하며, consumer는 그 provenance를 각 KnowledgeUnit에 복사하면 안 됩니다.

Source 변경이 KnowledgeUnit에 암묵적으로 반영되면 안 됩니다. 작성자는 새 SourceRevision을 명시적으로 선택하고 pin을 갱신해야 하며, 그 결과 새로운 Knowledge RevisionId가 만들어집니다. 새 revision이 신뢰받는 권위가 되려면 review, exact-revision approval, Checkpoint activation을 받아야 합니다.
