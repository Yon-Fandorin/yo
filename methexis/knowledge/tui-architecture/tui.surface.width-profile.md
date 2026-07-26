---
schema: methexis.knowledge/v1alpha1
id: tui.surface.width-profile
kind: decision
owner: tui-architecture
sources:
  - id: tui.surface-005
    revision: sha256:0bbc68263301ea99e4cd145491062b965e3ecc168dc0b429446c8b6613b9bfc2
relations:
  depends_on:
    - tui.surface.text-segmentation
  constrained_by:
    - tui.crate.ui-only-boundary
  validated_by:
    - surface.width-profile-fixtures
  applies_to:
    - yo-tui::surface::width
---
# Versioned cell-width profile

## Statement

Terminal and HTML projection MUST use the exact profile
`yo-unicode-17.0-narrow/v1`, based on Unicode 17.0 data. It accepts one complete
extended grapheme cluster and returns only width one or two:

1. a cluster containing a standardized text presentation sequence listed in
   Unicode Emoji 17.0 `emoji-variation-sequences.txt` with `VS15` uses the
   non-emoji rule, overriding the scalar's default emoji presentation;
2. otherwise, an RGI emoji sequence from Unicode Emoji 17.0, a scalar with
   `Emoji_Presentation=Yes`, or a cluster containing a standardized emoji
   presentation sequence from that file with `VS16` has width two;
3. otherwise, combining marks, variation selectors, ZWJ, and default-ignorable
   scalars contribute zero; East Asian Width `W` or `F` scalars contribute two;
   `A`, `H`, `Na`, and `N` scalars contribute one; and cluster width is the
   maximum contribution.

A `VS15` or `VS16` occurrence that is not part of a standardized variation
sequence has no presentation effect and contributes zero under rule 3.

An all-zero cluster MUST be rejected as `ZeroWidth`. Newline and tab belong to
text layout, and other control characters MUST be rejected before cell
mutation. Inputs containing more than one grapheme cluster MUST be rejected.

Future terminal capability detection MAY select another supported profile, but
the selected identity MUST be recorded. A profile change MUST invalidate the
completed frame and force a full redraw; it MUST NOT silently reinterpret
existing cells.

## Rationale

Pinned segmentation, Unicode data, emoji handling, and width resolution prevent
two conforming adapters from producing different occupancy. Explicit profile
replacement leaves room for environment-aware behavior without making current
rendering depend on hidden terminal state.
