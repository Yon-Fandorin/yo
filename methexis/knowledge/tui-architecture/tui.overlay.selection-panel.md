---
schema: methexis.knowledge/v1alpha1
id: tui.overlay.selection-panel
kind: decision
owner: tui-architecture
sources:
  - id: tui.overlay-001
    revision: sha256:bf952e72e8c70e000f0177c3c009eaa39d8450d5a8f97a55436aaed0c3b0a935
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
row. A row is either a non-selectable section, a non-selectable status owned by one section, or a selectable entry. Section identities and entry identities MUST each be unique within one snapshot. Each section MUST contain an opaque stable identity and non-empty label. Each status MUST name its owning section and contain non-empty status text. Each entry MUST name its owning section, contain an opaque stable identity, a non-empty primary label, optional detail, typed current state, and typed availability of either enabled or disabled with a reason. Title,
captions, labels, detail, and disabled reasons MUST pass the existing safe
grapheme and control-text validation before publication. Navigation MUST skip sections, statuses, and disabled entries, and accept MUST never return any of their identities. An
all-disabled snapshot MUST remain displayable with no selection and accept MUST
return a handled no-selection outcome.

One provider-controlled snapshot-level interaction gate MUST be independent of
per-entry availability. A fresh snapshot MAY issue one acceptance receipt for
its enabled selection. A pending-replacement snapshot MUST preserve the
entries, selected identity, and entry styling supplied by the last fresh
snapshot, but `Tab` and `Enter` MUST be handled without issuing a receipt or
submitting the draft. Returning to fresh state MUST preserve the selected
identity when it is still enabled. The panel MUST NOT represent snapshot
freshness by changing an entry's semantic availability. While destination
geometry is unchanged, the pending viewport MUST remain stable. Resize MUST
apply normal fitting, selection visibility, and insufficient-geometry hiding
rules. This is the panel's only semantic interaction gate. The panel MUST NOT
own or infer whether its caller has committed a rendered frame;
synchronization between a logically refreshed snapshot and its visible
presentation belongs to the prompt-slot routing owner and MUST NOT be
reimplemented by a provider.

Optional title status MUST be typed as static or activity presentation; render
code MUST NOT infer activity by parsing its text. The provider or controller
owns the semantic state and safe status text. The panel owns only validated
presentation and MAY apply an appearance-resolved style-only sheen to activity
status without changing its text or geometry.

The panel MUST span the prompt width and use a muted frame, a title at the left
of the top border, compact current-binding hints at the right, bold section labels, an
appearance-profile-resolved marker and accent focus treatment on the selected
entry row, muted optional detail and status rows, dimmed disabled rows with their reason, and
explicit hidden-above or hidden-below counts. Section and status rows consume viewport space but never receive the selection marker. When a provider filters entries, it MUST retain each matching entry's owning section and remove empty sections. A provider MAY append literal safe text ` (current)` to a current entry's primary label; the panel MUST render it inline without inventing a leading separator or separate status column. Selection MUST remain visible as
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
Separating semantic replacement freshness from caller-owned frame
synchronization keeps one owner for each gate and prevents individual providers
from racing visible selection.
