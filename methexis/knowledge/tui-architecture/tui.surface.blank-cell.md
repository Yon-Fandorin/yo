---
schema: methexis.knowledge/v1alpha1
id: tui.surface.blank-cell
kind: decision
owner: tui-architecture
sources:
  - id: tui.surface-013
    revision: sha256:974b8e7e187690f896f61d5f455c5935e52e0689bf094327252aa8374ed1dd1e
relations:
  depends_on:
    - tui.surface.resolved-style
  validated_by:
    - surface.blank-cell-tests
  applies_to:
    - yo-tui::surface::cell
---
# Explicit blank cell

## Statement

`Blank` MUST be an explicit cell state with no grapheme occupancy and one
resolved `Style`. A newly created or explicitly reset `Surface` MUST fill cells
with `Blank` using terminal-default foreground and background and no
attributes.

A clear operation MUST take an explicit resolved `Style`. Cleanup performed as
part of a grapheme overwrite MUST use the incoming write style for every
vacated cell, preserving its background and attributes without retaining old
grapheme ownership.

## Rationale

An explicit styled blank makes initialization, clearing, diff, and HTML parity
observable. Reusing the incoming style during overwrite avoids a hidden choice
between ghost content and an unrelated default background.
