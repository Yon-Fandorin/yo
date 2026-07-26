---
schema: methexis.knowledge/v1alpha1
id: tui.terminal.lifecycle-restoration
kind: decision
owner: tui-architecture
sources:
  - id: tui.terminal-002
    revision: sha256:d1ec20f7d7dfb4701076011cc3df266d2df05755f00f141aa53ab7cb43add8c8
relations:
  depends_on:
    - tui.runtime.typed-flow
    - tui.surface.terminal-ops
    - tui.terminal.inline-viewport
  constrained_by:
    - tui.architecture.module-boundaries
  validated_by:
    - terminal.entry-rollback-tests
    - terminal.mode-restoration-tests
    - terminal.signal-restoration-tests
  applies_to:
    - yo-tui::terminal::mode
---
# Transactional terminal lifecycle

## Statement

The outer mode controller MUST capture the original TTY input state before its
first mutation. Each lifecycle or mode-acquisition mutation MUST register its
compensating cleanup before bytes or system state can change, so a partial
write or uncertain result still causes the inverse action to be attempted.
Ordinary frame writes do not create per-write compensation entries. Inline and
Fullscreen use the same lifecycle engine; only Fullscreen registers
alternate-screen ownership.

Normal exit, entry failure, rendering failure, a panic crossing the terminal
session boundary, and configured Unix termination signals MUST converge on one
idempotent explicit restoration path. The terminal-owning session boundary MUST
catch an unwind, run that path, and then resume the original unwind. The
controller MUST temporarily route panic reporting while it owns terminal state
so diagnostic metadata is retained but is not emitted into the mutable or
alternate screen. It MUST restore the preceding panic hook and emit the retained
diagnostic only after terminal restoration, before resuming the unwind. The
restoration path MUST stop terminal producers, finish or cancel any active
synchronized update, reset resolved style, restore captured cursor properties
when reliable, otherwise make the cursor visible and reset its shape to the
terminal default, disable registered input or output modes in reverse
acquisition order, leave the alternate screen when owned, and finally restore
the captured TTY state. Every applicable cleanup MUST be attempted even when an
earlier cleanup fails.

Inline restoration MUST clear only its provably owned active viewport and leave
the cursor immediately below persistent output. Fullscreen restoration MUST
return to the intact main screen. The controller MUST return a structured
session outcome; any optional exit summary is emitted by the caller only after
restoration.

Configured asynchronous termination signals MUST enter the typed control path
and perform cleanup on the terminal-owning thread rather than writing terminal
sequences directly inside a signal handler. After cleanup, the process MUST
remove its installed notification handling, unblock the terminating signal, and
re-raise that same signal under its default disposition; it MUST NOT substitute
a numeric exit code. `SIGKILL`, synchronous fatal faults, and process abort are
outside the restoration guarantee.

Job-control suspension and resumption are not termination exits and are outside
this initial contract. Supporting them requires a separate restore-before-stop
and transactional reacquisition plus full-redraw contract.

Explicit restoration MUST report the primary failure and all cleanup failures
without allowing cleanup to mask the cause. `Drop` MUST provide an idempotent,
non-panicking best-effort fallback for unwind and early-return mistakes, but it
MUST NOT replace the reportable explicit path.

## Rationale

Terminal acquisition can fail after visible state has already changed, and
cleanup itself can also fail. Pre-registered compensation and one owner-thread
shutdown path cover partial entry, raw-mode signal behavior, and panic without
pretending that `Drop`, a signal handler, or one reverse escape sequence can
provide complete restoration evidence.
