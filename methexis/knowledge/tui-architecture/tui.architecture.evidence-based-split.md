---
schema: methexis.knowledge/v1alpha1
id: tui.architecture.evidence-based-split
kind: decision
owner: tui-architecture
sources:
  - id: tui.arc-004
    revision: sha256:1084e2f78f3f92cb38ef03387f7bc39edeb42db24b53a3ea665507f284a3f0a9
relations:
  constrained_by:
    - tui.crate.ui-only-boundary
  validated_by:
    - architecture.split-review
---
# Evidence-based crate split

## Statement

Production code MUST remain in one `yo-tui` crate until an independent
production consumer demonstrates a shared contract, dependency boundary, and
release cadence. A future Tauri application MAY justify extracting stable
domain semantics but not terminal-specific UI structures.

## Rationale

Splitting before a second consumer exists creates public boundaries from
speculation and makes later changes harder without proving independent value.
