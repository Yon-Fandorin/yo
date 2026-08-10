---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.source.kind-vocabulary
revision: sha256:6ef6fd9cb2d728bdc0e36239b1cef51ce147f13b18c6868ed511bc20e62dad6d
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:d746cd9f799b76183d0cc7c4d9c55f3f6ae2cae3ca6a146a2e28d6e7fa1c215b
---
# Korean Review Projection

## Translation

# Source 종류 어휘

## 선언

초기 Source 어휘는 다음 네 종류로 닫혀 있습니다.

- `decision`은 수락된 설계 결정을 기록합니다.
- `code`는 저장소 경로, symbol, 정확한 content hash를 기록합니다.
- `conversation`은 허가된 최소 발췌문 또는 opaque reference를 기록합니다.
- `external`은 저장소 외부의 문서 또는 표준을 기록합니다.

Conversation material은 허가된 발췌문 또는 content hash가 있는 opaque reference 중 하나여야 합니다. External freshness는 immutable, mutable, attested 중 하나로 선언해야 합니다. 해당 종류와 freshness mode의 verifier가 존재할 때까지 Conversation과 External Source는 ineligible 상태로 남아야 합니다.

canonical English body는 agent가 만들고 Draft로 시작합니다. 한국어 사용자 입력이 material provenance이면 reviewer는 authorized Source excerpt와 생성된 Korean review Projection을 봅니다. 전체 transcript는 기본 보관하지 않으며 tracked conversation Source는 최소 관련 excerpt, sensitive content redaction, 명시적 human authorization을 요구합니다. Sensitive provenance는 opaque reference와 content hash로 Git 밖에 둘 수 있고 English 효율은 영구 전제가 아니라 측정할 Pilot 가설입니다.

### 전체 개정 정본 원문 대조

# Source kind vocabulary

## Statement

The initial Source vocabulary is closed to these four kinds:

- `decision` records an accepted design decision;
- `code` records a repository path, symbol, and exact content hash;
- `conversation` records an authorized minimal excerpt or an opaque reference;
- `external` records a document or standard outside the repository.

Conversation material MUST be either an authorized excerpt or an opaque
reference with a content hash. External freshness MUST be declared as
immutable, mutable, or attested. Conversation and External Sources MUST remain
ineligible until a verifier for the corresponding kind and freshness mode
exists.

The canonical English body is agent-generated and begins as Draft. When Korean
user input is material provenance, a reviewer sees an authorized Source excerpt
and a generated Korean review projection. Full transcripts MUST NOT be retained
by default. Tracked conversation Sources contain only a minimal relevant
excerpt, redact sensitive content, and require explicit human authorization.
Sensitive provenance MAY remain outside Git behind an opaque reference and
content hash. English efficiency is a measured Pilot hypothesis, not a
permanent product assumption.
