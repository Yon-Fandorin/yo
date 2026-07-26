---
schema: methexis.knowledge/v1alpha1
id: tui.terminal.inline-viewport
kind: decision
owner: tui-architecture
sources:
  - id: tui.terminal-001
    revision: sha256:09566a1e3f4602e0cf492602d3fbb116ca1199398b30a7b6b06c99f2c5f8cac7
relations:
  depends_on:
    - tui.surface.terminal-ops
  constrained_by:
    - tui.architecture.module-boundaries
  validated_by:
    - terminal.inline-viewport-fixtures
    - terminal.inline-resize-tests
  applies_to:
    - yo-tui::terminal::mode::inline
---
# Owned inline viewport

## Statement

Inline mode MUST render on the main screen and preserve terminal scrollback. It
MUST own one active viewport followed immediately by a cursor anchor. Surface
coordinates are logical coordinates relative to that anchor; ordinary
rendering MUST NOT require an absolute cursor-position query.

In steady state, the controller owns the whole physical rows allocated to the
current Surface height. During a height transition, the previous footprint
remains owned until the controller reconciles the maximum of the previous and
current heights and moves the anchor immediately below the new viewport.
Completed output MAY be inserted above the active viewport and then becomes
persistent scrollback outside the controller's mutable region.

A terminal geometry change MUST invalidate the previous frame. While the
anchor and whole-row ownership remain provable, the controller MUST redraw the
latest completed Surface in place; otherwise it MUST use the recovery below.
Replaceable intermediate resize states MAY be coalesced. Ordinary resize MUST
NOT create a persistent snapshot.

If the controller can no longer prove its anchor or physical row ownership, it
MUST NOT erase outside the provable region. It MUST abandon that region,
re-anchor below it, perform a full redraw, and expose the recovery as
environmental evidence rather than a deterministic success.

## Rationale

A bounded mutable viewport gives agent interaction a stable composer while
keeping completed work available through native scrollback. Relative ownership
avoids a mandatory terminal response protocol, and the explicit recovery path
prefers duplicated diagnostic output over deleting user-owned terminal
history.
