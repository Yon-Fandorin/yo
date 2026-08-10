---
schema: methexis.review-projection/v1alpha1
knowledge_id: librarian.discovery.advisory-boundary
revision: sha256:18c1675f3c19be11b3f83d299c05957f1bf5ff445ac57c7f28df52342206968e
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:f84065fa8da3882873610713750f42bdd2611648f8fddd5ea2c402037eccabb5
---
# Korean Review Projection

## Translation

Librarian은 후보와 근거, 위치, 분류 및 이동 계획을 제안할 수 있지만 승인·활성화·안전성을 선언하거나 Checkpoint를 우회할 수 없습니다. 빈 요청은 실패하고, 미해결 anchor나 검색 결과 없음은 명시적인 성공 관찰입니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

Final context selection is deterministic. Librarian is an advisory discovery
and catalog component that MAY:

- propose candidate KnowledgeIds and explain each reason;
- map stable semantic IDs to physical locations;
- recommend classification and placement;
- detect duplicates, orphans, and broken references;
- generate reviewable relocation plans.

Librarian MUST NOT approve meaning, mutate canonical authority silently, or
bypass a Checkpoint. Search and LLM output are candidate signals only.

The first agent path accepts a versioned request containing at least one of a
non-empty natural-language query or one or more code-path, symbol, and
KnowledgeId anchors. It returns a deterministic, versioned candidate set.
Each candidate contains a stable KnowledgeId and machine-readable reasons;
Librarian never labels it approved, active, or safe to use. Methexis owns
required-closure expansion and final eligibility filtering.

An unresolved anchor and a query with no matches are successful observations
and remain explicit in the result. A request with neither query nor anchors is
invalid. An invalid catalog produces a structured failure and no partial
candidate set; silently skipping a damaged record could hide required
knowledge.

The discovery command writes exactly one complete structured success value to
stdout. It writes a structured failure to stderr, leaves stdout empty, and
returns non-zero. Callers MAY pipe or redirect successful JSON to a file; the
Pilot MUST NOT create or own a persistent candidate artifact or cache. The
result identifies the request, catalog snapshot, compiler, ordered candidates,
reasons, unresolved anchors, and truncation. S4 hashes the exact candidate input
it consumes into the ContextBuild lineage.
