---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.appearance.scoped-preview
revision: sha256:e4f1507c9bf618e11c5f389cf0e04019519504dfc4ffadd35a2f347214356335
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:b5db9659cf7d57a6b5b3265d062e429b2522b55e8b89c55806f93fbeeb03e1ff
---
# Korean Review Projection

## Translation

settings preview는 draft appearance snapshot을 소유한 preview subtree에만 명시적으로 전달해야 합니다. transcript, prompt, settings chrome과 그 밖의 subtree는 계속 committed snapshot을 받아야 합니다. preview state가 session의 committed snapshot을 바꾸거나 임시로 대체하면 안 됩니다.

각 preview는 재사용하지 않는 generation-bearing opaque `PreviewId`를 사용하고, owner subtree 수명과 base committed revision에 묶어야 합니다. Cancel, owner close, owner error, suspend 시 preview를 폐기합니다. committed revision이 바뀌면 이전 revision 기반 preview는 stale 상태가 되며 조용히 rebase하지 말고 명시적으로 무효화해야 합니다.

Save는 owner transaction으로 실행해야 합니다. 관련 durable configuration baseline과 committed appearance revision을 재검증하고, persistence에 성공한 뒤에만 완전한 committed snapshot 하나를 다음 logical frame에 게시합니다. persistence 실패나 conflict가 나면 committed appearance를 보존해야 합니다.

preview glyph 변경은 preview subtree만 다시 측정하고, global commit은 logical frame 전체를 invalidate하고 다시 측정해야 합니다. 정확한 persistence baseline, conflict UI, durable failure ordering은 실제 settings storage owner와 함께 후속 계약으로 정합니다. 이 KnowledgeUnit은 preview 구현과 evidence가 생길 때까지 inactive로 유지해야 합니다.
