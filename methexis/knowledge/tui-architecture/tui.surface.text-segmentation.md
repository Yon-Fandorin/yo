---
schema: methexis.knowledge/v1alpha1
id: tui.surface.text-segmentation
kind: decision
owner: tui-architecture
sources:
  - id: tui.surface-012
    revision: sha256:cb44e4ac50682edbd3ab99755be0b7d2604681206fdf11285e207d7092ca95ec
relations:
  constrained_by:
    - tui.crate.ui-only-boundary
  validated_by:
    - surface.unicode-17-grapheme-fixtures
  applies_to:
    - yo-tui::surface::text
---
# Extended grapheme segmentation

## Statement

Text layout MUST segment input with the unmodified Unicode 17.0 extended
grapheme cluster boundary algorithm, UAX #29 conformance clause C1-1. It MUST
NOT apply locale or CLDR tailoring.

A segmented cluster MUST retain its original UTF-8 string. Segmentation MUST
NOT normalize or rewrite stored text; canonically equivalent strings may have
the same boundaries while remaining byte-distinct cell content.

## Rationale

Pinning the boundary algorithm and data version makes emoji ZWJ, regional
indicator, combining-mark, and script-specific clusters deterministic without
making one Rust dependency the semantic authority.
