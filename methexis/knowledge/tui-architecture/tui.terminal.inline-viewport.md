---
schema: methexis.knowledge/v1alpha1
id: tui.terminal.inline-viewport
kind: decision
owner: tui-architecture
sources:
  - id: tui.terminal-001
    revision: sha256:3766bb541da1b820aee0c033d7a7dda928b1ff0373e4173a3c07562c183087b7
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
MUST own one active viewport followed immediately by a logical bottom anchor.
Surface coordinates are logical coordinates relative to the viewport. Between
frames, the physical terminal cursor MAY rest at the prompt caret inside the
viewport so terminal-native input methods follow the visible caret. The
controller MUST remember that caret relative to the viewport and return to its
owned coordinate system with relative controls; ordinary rendering MUST NOT
require an absolute cursor-position query.

In steady state, the controller owns the whole physical rows allocated to the
current Surface height. During a height transition, the previous footprint
remains owned until the controller reconciles the maximum of the previous and
current heights and moves the anchor immediately below the new viewport.
Completed output MAY be inserted above the active viewport and then becomes
persistent scrollback outside the controller's mutable region.

The controller MUST hide the physical cursor while it redraws and reveal it at
the current prompt caret only after a complete frame is flushed. Cursor
visibility restoration MUST be registered with terminal lifecycle ownership
before drawing starts, so normal exit, rendering failure, and panic cleanup all
attempt to leave the cursor visible.

A terminal geometry change MUST invalidate the previous frame. While the
anchor and whole-row ownership remain provable, the controller MUST redraw the
latest completed Surface in place; otherwise it MUST use the recovery below.
Replaceable intermediate resize states MAY be coalesced. Ordinary resize MUST
NOT create a persistent snapshot.

If the controller can no longer prove its logical anchor, remembered caret, or
physical row ownership, it
MUST NOT erase outside the provable region. It MUST abandon that region,
re-anchor below it, perform a full redraw, and expose the recovery as
environmental evidence rather than a deterministic success.

## Rationale

A bounded mutable viewport gives agent interaction a stable composer while
keeping completed work available through native scrollback. Relative ownership
avoids a mandatory terminal response protocol, and the explicit recovery path
prefers duplicated diagnostic output over deleting user-owned terminal
history.
