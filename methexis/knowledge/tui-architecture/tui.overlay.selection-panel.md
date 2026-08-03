---
schema: methexis.knowledge/v1alpha1
id: tui.overlay.selection-panel
kind: decision
owner: tui-architecture
sources:
  - id: tui.overlay-001
    revision: sha256:de6d55d11499f4c9be696c941c3e55a778e635989568c153f9aec046ff9dd0b5
relations:
  constrained_by:
    - tui.surface.geometry
    - tui.surface.resolved-style
    - tui.surface.grapheme-cells
    - tui.appearance.frame-consistency
  applies_to:
    - yo-tui::overlay::selection
---
# Selection overlay panel

## Statement

The first reusable overlay presentation component MUST be a pure selectable
panel based on Rib's prompt completion panel. It MUST receive already validated
semantic entries and resolved binding hints. It MUST own selected-identity
viewport fitting and presentation and MUST return typed navigation outcomes.
It MUST NOT discover or filter candidates, own provider query or preview state,
access a filesystem or backend, or execute an accepted product effect.

The input MUST contain a non-empty safe title, current keymap-derived physical
binding labels paired with semantic action captions, and at least one ordered
entry. Entry identities MUST be unique within one snapshot. Each entry MUST
contain an opaque stable identity, a non-empty primary label, optional detail,
and typed availability of either enabled or disabled with a reason. Title,
captions, labels, detail, and disabled reasons MUST pass the existing safe
grapheme and control-text validation before publication. Navigation MUST skip
disabled entries, and accept MUST never return a disabled identity. An
all-disabled snapshot MUST remain displayable with no selection and accept
MUST return a handled no-selection outcome.

The panel MUST span the prompt width and use a muted frame, a title at the left
of the top border, compact current-binding hints at the right, an
appearance-profile-resolved marker and accent focus treatment on the selected
row, muted optional detail, dimmed disabled rows with their reason, and
explicit hidden-above or hidden-below counts. Selection MUST remain visible as
it moves. Wide layout SHOULD align primary and detail text as two columns.
Narrow fitting MUST remove detail and disabled reason before truncating the
primary label at a grapheme boundary. Content MUST NOT wrap outside the panel
or change the supplied width.

Available height MUST be bounded by the caller's destination rectangle and a
component-owned visible-entry cap. Geometry that cannot contain both borders
and one entry MUST produce a hidden outcome without panicking or painting.
Resize MUST preserve an enabled selected identity when it remains present and
recompute only the visible window. One pinned appearance revision MUST govern
measurement and paint. Validation, preparation, and paint MUST be atomic:
failure MUST leave the destination Surface and previously published panel
state unchanged.

## Rationale

Rib's completion and picker retain distinct state and effects but share a
recognizable panel language. Reusing that narrower presentation boundary gives
Yo consistent prompt-adjacent choices without forcing filesystem completion,
session resume, model preview, and later providers into one controller.
