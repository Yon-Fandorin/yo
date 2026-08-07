---
schema: methexis.knowledge/v1alpha1
id: tui.grapheme-cells
kind: decision
owner: tui-architecture
sources:
  - id: tui.fixture
    revision: sha256:3d3ff9057aadcbf2f44300bce0f97c5c84dc3c59a1a76e09eb012b299892f130
---
# Grapheme cell storage

## Statement

Terminal cells store exactly one grapheme cluster each.

## Rationale

Splitting clusters across cells corrupts cursor accounting.
