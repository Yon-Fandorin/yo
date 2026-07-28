---
schema: methexis.knowledge/v1alpha1
id: tui.architecture.evidence-based-split
kind: decision
owner: tui-architecture
sources:
  - id: tui.arc-004
    revision: sha256:f8142dd48cbea18f4703cb05e1bc991efce810f688b770617d591ec36a4c9f83
relations:
  constrained_by:
    - tui.crate.ui-only-boundary
  validated_by:
    - architecture.split-review
---
# Evidence-based crate split

## Statement

`yo-tui` MUST remain the single production TUI library until an independent
consumer demonstrates a shared contract, dependency boundary, and release
cadence. A product entry package such as `yo-cli` MAY compose that library and
own process-wide policy; this necessary executable host is not a speculative
extraction of shared TUI or domain internals. A future Tauri application MAY
justify extracting stable shared semantics but not terminal-specific UI
structures.

## Rationale

Splitting before a second consumer exists creates public boundaries from
speculation and makes later changes harder without proving independent value.
