---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.knowledge.semantic-continuity
revision: sha256:d299d81adfefdbd2bf61449ef6e1e08138d468dc7018cb20a362318066e61921
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:564d42e4ea101904d125ec3db05be179c10e45589ed5f332a3b05a481ed60aed
---
# Korean Review Projection

## Translation

# Knowledge 의미 연속성

## 선언

revision은 동일한 의미 질문에 답하고 기존의 모든 inbound relation이 계속 동일한 의무를 가리킬 때에만 같은 `KnowledgeId`에 남아야 합니다. 같은 의무에 대한 설명 보완, 더 엄격한 표현, 결과 변경은 기존 ID의 새 revision입니다.

기존 inbound relation의 의미를 조용히 바꿀 정도로 주제나 의무가 달라지면 `supersedes`로 연결한 새 `KnowledgeId`를 사용해야 합니다. 모든 supersession target은 존재해야 하고 supersession graph는 acyclic해야 하며, 이전 unit과 replacement는 함께 active이면 안 되고, 제거되는 ID 때문에 필수 inbound relation이 미해결 상태가 되면 안 됩니다.

결정론적 validation은 이 구조적 보장만 확립합니다. Librarian은 겹치는 anchor나 유사한 의미를 설명되지 않은 replacement 가능성으로만 표시할 수 있으며, 사람 reviewer가 의미 연속성 결정의 소유자여야 합니다.
