---
schema: methexis.knowledge/v1alpha1
id: tui.dependencies.terminal-backend-selection
kind: decision
owner: tui-architecture
sources:
  - id: tui.dependencies-001
    revision: sha256:86e9d5a7077db91c100f3da00d773be2f9697290f6ae8399393635ca767f65bf
relations:
  depends_on:
    - tui.terminal.lifecycle-restoration
  constrained_by:
    - tui.architecture.module-boundaries
    - tui.dependencies.selection-gate
  validated_by:
    - terminal.backend-mock-tests
    - terminal.unix-compile-matrix
    - terminal.environment-matrix
  applies_to:
    - yo-tui::terminal::backend
    - yo-tui::terminal::input
---
# Minimal Unix terminal backend

## Statement

The initial macOS and Linux terminal adapter MUST use these exact dependency
surfaces:

- `crossterm = 0.29.0` with default features disabled and only `events` plus
  `bracketed-paste` enabled, for synchronous input polling and decoding of key,
  paste, focus, mouse, and resize events;
- `rustix = 1.1.4` with default features disabled and only `std`, `stdio`, plus
  `termios` enabled, matching Crossterm's unavoidable Unix Rustix surface and
  providing exact original TTY capture, raw-input mutation, and restoration;
- `signal-hook = 0.3.18` with default features disabled and only `iterator`
  enabled, for forwarding configured asynchronous Unix termination signals to
  the typed control path and emulating their default disposition after
  restoration.

These dependencies MUST be target-scoped to Unix. The first adapter MUST NOT
enable Crossterm's `event-stream`, `serde`, `windows`, `derive-more`, `osc52`,
or `use-dev-tty` features, Rustix's PTY or unrelated syscall features, or
Signal Hook's extended signal-information features. `signal-hook` remains on
the latest `0.3` patch to share Crossterm's compatible dependency graph; moving
to `0.4` requires a separate compatibility review.

Yo MUST retain ownership of `Surface`, `FrameDiff`, `TerminalOp`, ANSI output,
mode acquisition order, cleanup policy, and public input events. Crossterm,
Rustix, and Signal Hook types MUST terminate inside a crate-private
`terminal::backend` adapter. Deterministic lifecycle tests MUST use the same
internal backend trait with a recording fake rather than a live terminal.

Acceptance MUST include macOS and Linux compilation, deterministic backend and
partial-failure tests, and environmental evidence for a local modern terminal,
tmux, an SSH PTY, and tmux inside that SSH session. Environment-unavailable
matrix entries MUST remain explicit rather than being reported as deterministic
failures.

## Rationale

Crossterm alone has the desired mature event decoder, but its raw-mode helper
stores the original termios state only after the mutation reports success, which
cannot satisfy the pre-registered compensation contract for an uncertain
partial failure. Rustix supplies the narrow, I/O-safe termios operations needed
for exact capture and restoration without requiring a custom escape-sequence
parser. Signal Hook supplies deferred signal delivery and same-signal default
emulation without terminal writes inside the asynchronous handler.

A Rustix-only adapter would require yo to build and maintain modern keyboard,
paste, mouse, focus, and resize parsing. Termwiz duplicates yo's existing
Surface, cell, style, and renderer ownership and documents a comparatively
unstable API. Termina is the closest modern replacement candidate because its
lower-level parser keeps terminal protocols visible, but its still-young
pre-1.0 adapter surface has less accumulated compatibility evidence than
Crossterm and also exposes escape, style, and terminal ownership that yo does
not need. The selected split keeps those larger policy surfaces out of the
dependency boundary while leaving each library replaceable behind typed yo
values. Termina SHOULD be reconsidered through a separate compatibility review
if the initial environment matrix exposes a Crossterm limitation.
