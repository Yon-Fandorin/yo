---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.graduation.repository-boundary
revision: sha256:d5a5cadc5ddcd1d061e861d405e52b04a0052d8648be0498d4d166797924c55f
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:22b39e9d163c51a9475574555334790b65a16fde4365852c4e85b9d50b6af855
---
# Korean Review Projection

## Translation

Librarian과 Methexis가 독립 저장소로 졸업하려면 계약·fixture·평가가 이전되고 yo가 standalone 구현을 소비하면서 동일 평가를 통과해야 합니다. 두 번째 실제 제품 소비자 전에는 yo에서 검증된 계약을 넘어 일반화하지 않습니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

Librarian graduation requires the same contract to work for both Surface and
the SOT operating-procedure corpus, no `yo`-specific public types, identity
preservation across relocation, no authority mutation through search,
transferred contract tests, and a passing `yo` Pilot against the final
Librarian.

Methexis repository graduation requires:

- the deterministic suite and A/B/C Pilot evaluation pass in `yo`;
- its public contract contains no TUI- or `yo`-specific types;
- contract, fixture, and failure tests transfer to the standalone repository;
- `yo` passes the same evaluation while consuming standalone Methexis;
- the in-repository implementation shrinks to a thin adapter.

Repository extraction MAY happen after stable `yo` Pilot evidence when the tool
needs an independent release lifecycle. Until a second real product consumer
exists, the standalone project MUST NOT generalize beyond the contract proven
by `yo`.
