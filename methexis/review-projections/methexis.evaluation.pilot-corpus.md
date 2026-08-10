---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.evaluation.pilot-corpus
revision: sha256:eccf1bea10a74c2682b4d763176287c989ffc4ba396d2af576e4d722ac2ddc9a
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:196f5dabe09c5d196bb4c913e7ac9ae46d83aa6fd8ac73ae7e3201d10e4954e7
---
# Korean Review Projection

## Translation

Surface 20~50개 KU와 실행 증거가 준비된 뒤 동일한 깨끗한 저장소 상태에서 A/B/C 평가를 수행합니다. 필수 지식 회수율, 비활성 지식 노출, 성공률, 토큰, 변경 범위와 캐시 재사용을 측정하며, 보편적인 토큰 절감률을 미리 정하지 않습니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

The first evaluation corpus is the Structured Core `Surface` vertical slice. It
SHOULD contain roughly 20–50 units covering geometry, cell width, graphemes,
style, `Surface` invariants, common Inline and Fullscreen output semantics, HTML
projection, fixtures, and validation.

S5 begins with a smaller contract batch for independently reviewable model and
adapter decisions. Implementation Slices add fixture, failure, layout, and
mode-behavior units until the complete Surface corpus reaches the 20–50 range.
The A/B/C evaluation MUST NOT begin while that corpus shape or its executable
evidence is incomplete.

S1 precedes that evaluation corpus with five TUI architecture units that began
as Draft and were later reviewed, approved, and activated. This small seed
exists to exercise the file model, identity algorithm, graph validation, and
agent-facing Fast Check before the larger Surface authoring cost. It is
foundation evidence, not the evaluation corpus or a replacement for the S5
Surface gate.

The deterministic suite requires:

- identical inputs produce the same BuildId;
- required knowledge recall is 100%;
- exposure for every combination other than `approved` plus `active` is zero;
- changes invalidate only affected results;
- missing required knowledge fails explicitly;
- cached builds are reused.

Run 8–12 representative agent tasks under:

```text
A. existing full developer documentation
B. Librarian search results only
C. Librarian plus SOT ContextBuild
```

The first Surface run uses ten tasks. Six deterministic microtasks cover:

1. overwriting a wide grapheme without orphaning continuation cells;
2. atomically clipping a wide grapheme at the right boundary;
3. emitting the exact diff for a resolved-style change;
4. preserving global row-and-column diff ordering;
5. restoring cursor and terminal state after Inline mode;
6. preserving semantic parity between a completed Surface and its HTML
   projection.

Four real agent tasks cover:

1. adding one bounded Surface operation;
2. rendering one component through `SurfaceView`;
3. extending the typed terminal adapter;
4. diagnosing an injected fixture failure and adding its regression test.

Each A/B/C attempt starts from the same clean repository state and uses the
same task acceptance tests. The evaluator records unavailable environmental
matrix entries separately from deterministic task failures.

Measure task pass rate, tests passed, required-constraint recall, stale
exposure, successful-input tokens, unrelated file changes, and cache reuse.
Condition C must not reduce task pass rate relative to A, must reduce successful
input tokens, and must reduce required-rule omissions relative to B. Do not set
a universal token-reduction percentage before the Pilot establishes a baseline.
