---
schema: methexis.knowledge/v1alpha1
id: tui.appearance.glyph-profiles
kind: decision
owner: tui-architecture
sources:
  - id: tui.appearance-003
    revision: sha256:cdfe365a90cce5b5539d03d712ce08f4a3e1ec80985343b28ee18dd06c140186
relations:
  depends_on:
    - tui.appearance.session-publication
    - tui.appearance.frame-consistency
  constrained_by:
    - tui.surface.width-profile
  applies_to:
    - yo-tui::appearance::glyphs
    - yo-tui::components::transcript
---
# Explicit glyph profiles

## Statement

The initial appearance vocabulary MUST provide these exact transcript markers:

| Role | Rich | ASCII |
| --- | --- | --- |
| user | `❯` (`U+276F`) | `>` (`U+003E`) |
| assistant | `•` (`U+2022`) | `*` (`U+002A`) |

`Rich` MUST remain the default profile. `Ascii` MUST be selected only by
an explicit session appearance candidate. The initial implementation MUST NOT
infer a profile from `TERM`; color capability and `NO_COLOR` MUST NOT select a
glyph profile.

Changing the Rich assistant marker to `•` is an intentional presentation
change for pre-release consumers that compare rendered screen or plain-output
bytes. Message roles remain semantic input to rendering; a selected appearance
snapshot supplies their current markers rather than preserving an earlier
rendered marker as durable identity.

Every candidate marker MUST be one non-empty extended grapheme cluster and
MUST reject controls, ANSI content, and zero-width clusters before publication.
Measurement MUST use the existing `yo-unicode-17.0-narrow/v1` Surface width
owner rather than an appearance-specific width table. Every accepted marker
MUST fit within the configured body indent.

Rich and ASCII markers need not have equal cell width. Layout MUST compensate
inside the common indent so user and assistant body text begins at the same
configured column in both profiles. Screen and plain session output MUST obtain
their markers from the same committed snapshot.

## Rationale

The Rich profile keeps user input visually prominent while the quieter
assistant bullet lets response text carry the hierarchy. The explicit ASCII
path remains deterministic, and neither profile couples color preferences or
unreliable environment heuristics to terminal glyph capability.
