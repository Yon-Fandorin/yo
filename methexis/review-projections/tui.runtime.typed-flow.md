---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.runtime.typed-flow
revision: sha256:191d3c5030c6e2e161556232cd548bccf8b375cb52a85a586db14fb6aa6dac49
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:8a7e1136794c7301ba3b18ab52a48107335355766b57f989d41a79b475ed90ef
---
# Korean Review Projection

## Translation

런타임 통신은 이름이 있는 타입화된 event와 intent를 사용해야 합니다. 제어 이벤트는 용량이 제한되고 유실되지 않는 lane을 사용하며, 교체 가능한 상태 업데이트는 순서와 공정성을 위반하지 않는 명시적 coalescing을 사용합니다.

타입화된 lane은 과부하 동작을 검토 가능하게 만들고, 중요한 lifecycle 또는 approval 신호가 교체 가능한 UI 상태와 함께 조용히 버려지는 일을 막습니다.
