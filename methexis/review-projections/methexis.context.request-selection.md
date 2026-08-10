---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.context.request-selection
revision: sha256:69ec613684880d88595b11574ddcef31f59e0a7a05675bf26a7d268b87928b30
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:26b17eace3bc317e01bfb00d299ce40fa87824e37b334bcbb3a24bbb0d549161
---
# Korean Review Projection

## Translation

Context resolver는 pinned trusted commit과 active Checkpoint에서만 direct anchor를 해석합니다. 요청에는 direct anchor 또는 hash로 고정된 Librarian candidate reference가 필요하고, direct anchor는 필수 root이며 Draft나 fuzzy text는 선택 근거가 아닙니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

Context selection starts from explicit paths, symbols, KnowledgeIds, and
Librarian candidates. The resolver then:

1. resolves one immutable trusted integration commit;
2. loads the tracked active Checkpoint from that commit;
3. captures current local Source bytes and identities into an immutable
   Source snapshot;
4. verifies its cheap freshness guard;
5. filters by active Checkpoint eligibility;
6. expands required and constraining relations;
7. attaches applicable validation evidence;
8. applies priority and token-budget packing;
9. final-revalidates observed mutable Sources;
10. publishes an immutable, traceable `ContextBuild`.

The versioned request MUST contain at least one direct anchor or one Librarian
candidate result reference. A candidate reference is a repository-relative
local path plus the expected SHA-256 of the exact file bytes; the candidate
JSON is not embedded in the request. The resolver captures and verifies those
bytes before parsing and records their hash in ContextBuild lineage. A direct
KnowledgeId, path, or symbol anchor is a required root. An unresolved direct
path or symbol fails explicitly; when it resolves to multiple exact units, all
of them are required roots. Librarian candidates are advisory optional inputs.

Direct anchors resolve only against the KnowledgeSnapshot loaded from the
pinned trusted commit. A KnowledgeId matches its exact semantic ID. A path
matches either the exact canonical repository-relative Knowledge record path or
an exact `applies_to` value; a symbol matches only an exact `applies_to` value.
Code Source locators, Librarian's working-tree catalog, Draft files, and fuzzy
text do not participate. Anchor values use the same typed, trimmed
duplicate-rejection semantics as Librarian requests; the S4 request schema
additionally declares maximum anchor counts and value lengths.
