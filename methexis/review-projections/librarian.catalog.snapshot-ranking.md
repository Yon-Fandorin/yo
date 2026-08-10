---
schema: methexis.review-projection/v1alpha1
knowledge_id: librarian.catalog.snapshot-ranking
revision: sha256:ab96f1e47bde4c8cca91cd1617616232c5d25da2863b11754963384e5caf8e2b
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:ceee484827a0c63d5a1ec2361ad0b6a853b54fadb35a67df70905b141b5c62e0
---
# Korean Review Projection

## Translation

카탈로그는 working tree의 모든 구조적으로 유효한 KU를 한 번의 불변 snapshot으로 캡처합니다. 랭킹은 설명 가능한 결정적 lexical 증거만 사용하며 dependency closure 확장, vector/LLM/fuzzy ranking은 Pilot 기본값이 아닙니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

The initial catalog contains every structurally valid KnowledgeUnit, regardless
of approval or eligibility. Searchable fields are its ID, title, canonical
English body, typed relations, physical location, and an exact-revision valid
Korean review Projection when present. Source content, approvals, and
Checkpoints do not contribute text-ranking signals. Structured code Source
locators MAY satisfy an explicit path or symbol anchor without making Source
content searchable.

Librarian builds that catalog from the current working tree, including valid
untracked Draft records inside the declared corpus directories. It does not
resolve `develop` or grant trust to those files. It captures the sorted relative
paths and exact relevant bytes into one immutable catalog snapshot before
ranking. A concurrent change that prevents a consistent capture returns a
retryable failure and no candidate set.

Initial ranking is deterministic lexical evidence, ordered from exact ID and
anchor matches through phrase and token overlap to one-hop relation signals.
Every contribution remains inspectable in the candidate reasons. Librarian
MUST NOT expand required dependency closure. Semantic or vector retrieval, LLM
ranking, fuzzy matching, and language-specific morphological dependencies
remain evidence-gated extensions rather than Pilot defaults.
