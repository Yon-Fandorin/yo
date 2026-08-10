---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.workflow.self-hosting-boundary
revision: sha256:1c66fe9ee7a7df7ba5d5125b66a72a6aee742d834161dabba2389217de92094b
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:c6859215e51717378b440baf16c2d91fe09bad41a145a37c8b69760788658e4c
---
# Korean Review Projection

## Translation

Pilot 동안 CONTRIBUTING.md가 workflow의 유일한 정본입니다. 이를 KU 기반 DocumentView로 바꾸려면 완전한 coverage, 의미 동등성, 명시적 승인, drift check, 복구 수단과 이중 권위를 제거하는 atomic 전환이 모두 필요합니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

`CONTRIBUTING.md` remains the sole workflow authority during the Pilot. A future
explicit migration MAY make approved workflow KnowledgeUnits canonical and
commit `CONTRIBUTING.md` as their human-readable `DocumentView` Projection.

That migration requires:

- complete rule coverage and semantic-equivalence review;
- explicit human approval of the generated document;
- a generation-drift check in the repository validation path;
- a pinned last-known-good tool and documented recovery procedure;
- one atomic owner transition that removes dual authority.

After migration, contributors change the owning KnowledgeUnits and regenerate
the committed Projection rather than editing it independently. The generated
file remains readable without running Methexis.
