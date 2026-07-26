---
schema: methexis.knowledge/v1alpha1
id: tui.surface.deterministic-diff
kind: decision
owner: tui-architecture
sources:
  - id: tui.surface-008
    revision: sha256:512037b1212651047735ddd684910614a4088b49a259c377e8cdbf4fddbeb23a
relations:
  depends_on:
    - tui.surface.grapheme-cells
    - tui.surface.model-ownership
  validated_by:
    - surface.diff-fixtures
  applies_to:
    - yo-tui::surface::diff
---
# Deterministic completed-frame diff

## Statement

Diff MUST compare immutable completed previous and current `Surface` values and
emit changed row spans in ascending row and column order. Its result MUST
preserve grapheme boundaries and contain enough resolved cell state for an
adapter to render without consulting components.

The first implementation MUST compare full frames. Dirty-region tracking MAY
be introduced only after measurement and MUST preserve the same deterministic
result.

## Rationale

Completed-frame comparison provides a simple correctness oracle and stable
fixtures. Optimization can be checked against that oracle instead of defining
new semantics.
