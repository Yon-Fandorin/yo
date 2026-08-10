---
schema: methexis.knowledge/v1alpha1
id: methexis.evaluation.pilot-corpus
kind: rule
owner: methexis
sources:
  - id: methexis.evaluation-model.pilot-corpus
    revision: sha256:a889f4e6157c5f5f072d8d413c4ea989b6082538a485e2e6eb53098c632c36fc
---
# Pilot corpus and A/B/C evaluation

## Statement

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
