---
schema: methexis.knowledge/v1alpha1
id: tui.terminal.inline-viewport
kind: decision
owner: tui-architecture
sources:
  - id: tui.terminal-001
    revision: sha256:dfc55821103390a4075cc66c60d8a730b285f2e891e5c48706f0f2a765104a65
relations:
  depends_on:
    - tui.appearance.frame-consistency
    - tui.runtime.typed-flow
    - tui.surface.terminal-ops
  constrained_by:
    - tui.architecture.module-boundaries
  validated_by:
    - terminal.inline-viewport-fixtures
    - terminal.inline-resize-tests
  applies_to:
    - yo-tui::runner::prepare_frame
    - yo-tui::runner::session_output
    - yo-tui::terminal::mode::inline
---
# Compact inline live region

## Statement

Inline mode MUST render on the main screen and preserve terminal-native scrollback. The Chat projection MUST retain its complete ordered semantic history and separately own a monotonic scrollback publication cursor. A publication candidate MUST be the maximal contiguous prefix of unpublished `Final` Chat transcript items; it MUST NOT publish part of an item, skip an earlier `Streaming` item, or use visible row position as identity. The candidate boundary MUST identify the last item by stable transcript item identity and final revision.

For editable Chat in `FollowTail`, frame preparation MUST use one pinned appearance and the observed terminal width to compose both the candidate's persistent rows and a compact live Surface containing the unpublished suffix plus current prompt, chrome, and overlay. The composer MUST own separators and other formatting across the publication boundary. The live Surface height MUST be its measured natural height clamped to the available terminal height; it MUST NOT consume all available rows merely because they exist.

Publication is a prepare, present, observe, acknowledge transaction. Preparation MUST NOT advance the publication cursor. A prepared plan MUST bind the expected prior cursor, candidate boundary, appearance, complete observed terminal size, and a monotonic geometry epoch. The terminal-owning controller MUST advance that epoch for every resize notification it observes, including a notification whose size returns to an earlier value. The Inline presenter MUST insert the persistent rows immediately above the active viewport, reconcile the owned live footprint, place the prompt caret, and flush the complete plan before acknowledgement is considered. After flush, the controller MUST non-blockingly observe already-delivered resize notifications and sample terminal size again.

That post-flush observation MUST resolve persistent publication separately from live-viewport ownership. Persistent publication is complete only when every candidate operation was admitted to the terminal stream with grounded parser state and the effect ledger proves its complete effects. Live-viewport ownership is current only when the observed epoch and sampled size still match the prepared plan. When both are true, the controller MUST acknowledge the candidate and commit the live frame. When persistent publication is complete but geometry is stale, it MUST acknowledge the candidate boundary without clearing or replaying those persistent effects, preserve any rows already moved into native history, reject the prepared live frame and its physical ownership, and reprepare only the semantically unpublished live suffix and interactive chrome at the new geometry. That split acknowledgement asserts completed content delivery, not ownership of the old layout. When persistent publication is incomplete or cannot be proved, the semantic cursor MUST remain unchanged and the controller MUST enter the bounded effect-ledger reconciliation below; geometry mismatch alone MUST NOT replay a proven complete persistent prefix. A resize that the platform exposes only after this observation boundary belongs to the next observation, cannot retroactively change an acknowledged persistent prefix, and MUST invalidate the live frame and any physical ownership that can no longer be proved.

State MUST advance the publication cursor only for a complete persistent-publication receipt under the rule above. Only a matching whole-plan receipt also commits the prepared live frame. Preparation failure, a plan found stale before publication or lacking a complete persistent-publication receipt, or a write or flush failure MUST leave the semantic cursor unchanged and make uncertain physical ownership untrusted. On a proven successful transaction, each published item appears in native scrollback exactly once. When a partial terminal write makes success unknowable, the semantic cursor MUST remain unchanged.

Publication bytes MUST pass through a presenter-owned unbuffered terminal transport. Each reported write count MUST identify the exact byte prefix admitted to the downstream terminal stream, and `flush` MUST NOT newly transfer hidden buffered bytes. Writer acceptance above that transport is not delivery evidence. If the active transport cannot provide these properties, any write or flush failure MUST be treated as an unknown delivered prefix and MUST NOT enter erase-or-resume reconciliation.

The presenter MUST encode publication as ordered, self-delimiting terminal operations and retain, for the duration of the transaction, the expected operation bytes, expected cell rows, complete-operation terminal-stream progress, parser boundary, geometry epoch, anchor, cursor, and physical effects. The effect ledger MUST distinguish addressable main-screen writes and erasures from scrolling, insertion, deletion, or any operation that can move output into native history. Matching text alone MUST NOT establish ownership, and the design MUST NOT require non-portable terminal screen or scrollback readback.

A publication output error MUST preserve the original error as primary, leave the semantic cursor unchanged, and enter at most one bounded reconciliation attempt before it becomes a rendering failure. If the terminal-stream prefix ends at a complete operation with grounded parser state and the ledger still proves geometry, anchor, cursor, and ownership, the controller MUST choose the applicable exact correction rather than duplicate immediately. When every completed-prefix effect remains reversible inside currently addressable, provably owned main-screen rows, it MUST clear those effects and restart the complete prepared publication from that clean footprint. When an effect is irreversible but the exact completed prefix, resulting cursor, and remaining operation suffix are known, it MUST preserve that prefix without clearing or replaying it and resume only the remaining suffix. In both cases, only a complete recovered plan, flush, and matching post-flush geometry observation MAY acknowledge the candidate and continue the session; the recovery MUST be exposed as environmental evidence.

If exact correction is impossible but parser state is grounded and the controller can safely abandon the affected region and establish a fresh owned viewport, it MAY replay the complete semantically unpublished plan there; only this last-resort path may duplicate the earlier prefix. A successful replay MAY acknowledge its complete plan and continue the session. If an operation was admitted only partly, hidden lower-layer transfer is possible, parser state is ungrounded, a safe fresh viewport cannot be established, or the one recovery write or flush fails, the controller MUST stop recovery. The original publication error remains the primary rendering failure, reconciliation failure is attached as additional diagnostic evidence, and the existing terminal lifecycle restoration runs. No recovery path may erase unowned rows. These guarantees are semantic publication integrity, not guaranteed visible delivery across an unrecoverable process boundary.

Published scrollback rows are immutable terminal history and MUST NOT be reflowed, cleared, or replayed during an ordinary resize. The active viewport MUST relayout at the new width and reconcile the maximum of the previous and current provably owned heights before moving its logical bottom anchor. Replaceable resize states MAY be coalesced. If anchor, caret, or whole-row ownership is no longer provable, the controller MUST abandon that region, re-anchor below it, and fully redraw the unpublished live region without erasing persistent or unowned output.

While Chat is detached from the tail, or a read-only Transcript or Request view is active, Inline MUST freeze publication so navigation retains the complete visible history. Such a review viewport MAY use the available terminal height. Returning to editable Chat `FollowTail` MAY publish the newly eligible prefix and collapse back to the compact live height. Fullscreen MUST always render the complete semantic history, MUST ignore and never advance the Inline publication cursor, and MUST retain its existing screen and exit behavior.

Normal Inline exit and typed termination MUST first restore the terminal lifecycle and then expose only the unpublished retained Chat suffix as optional caller-owned session output. That suffix MUST use the same transcript formatting contract, including the separator at the publication boundary, and a committed appearance snapshot. A suspend transition MUST emit no session output, preserve the semantic publication cursor, and redraw the unpublished live region on the freshly acquired viewport after resume. If restoration on an otherwise eligible exit cannot prove whether the last physical publication completed, the exit suffix MUST still include every semantically unpublished item and MAY repeat visible output.

An output error fully recovered by the bounded reconciliation above is not a rendering-failure outcome. An unrecovered rendering failure, panic, and cleanup failure MUST retain the existing lifecycle failure and diagnostic disposition and MUST NOT be reclassified as a successful exit merely to emit optional session output. Those fatal paths are not required to emit the unpublished suffix; their guarantee is that an unacknowledged publication did not advance the semantic cursor before failure.

The controller MUST hide the physical cursor while it mutates the viewport and reveal it at the current prompt caret only after a complete plan is flushed. Cursor visibility restoration MUST be registered with lifecycle ownership before drawing starts. Ordinary rendering MUST use the remembered caret and relative controls and MUST NOT require an absolute cursor-position query.

## Rationale

Separating retained semantic history from a monotonic physical publication cursor lets Inline behave like a compact command-line conversation without weakening Fullscreen history or diagnostic navigation. Stable item identity, a post-flush geometry observation, and matching acknowledgement prevent state from claiming output that the terminal did not confirm under an observed layout. Treating published rows as immutable native history makes resize behavior predictable. On uncertain writes, exact terminal-stream and effect progress first enables clean restart or suffix-only continuation; only a grounded but unreconciled recovery falls back to duplicate replay rather than deleting user-owned terminal history. Ungrounded or repeated failures retain the established lifecycle diagnostics.
