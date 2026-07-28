---
schema: methexis.knowledge/v1alpha1
id: tui.dependencies.terminal-backend-selection
kind: decision
owner: tui-architecture
sources:
  - id: tui.dependencies-001
    revision: sha256:3bfc01fb7ae2f355d5918e080cba41c8b7e83ab6953633e71009f72b0cd784cb
relations:
  depends_on:
    - tui.runtime.process-termination-coordinator
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
    - yo-cli::process::termination
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
- `nix = 0.31.3` with default features disabled and only `signal` enabled, for
  typed signal masks plus exact capture and restoration of prior `sigaction`
  values in the private `yo-cli` process host;
- `signal-hook = 0.3.18` with default features disabled, only for its documented
  async-signal-safe emulation of a selected termination signal's default
  disposition.

These dependencies MUST be target-scoped to Unix. The first adapter MUST NOT
enable Crossterm's `event-stream`, `serde`, `windows`, `derive-more`, `osc52`,
or `use-dev-tty` features, Rustix's PTY or unrelated syscall features, Nix
features other than `signal`, or Signal Hook's iterator and extended
signal-information features. `signal-hook` remains on the latest `0.3` patch to
share Crossterm's compatible dependency graph; moving to `0.4` requires a
separate compatibility review.

Yo MUST retain ownership of `Surface`, `FrameDiff`, `TerminalOp`, ANSI output,
mode acquisition order, cleanup policy, and public input events. Crossterm and
Rustix types MUST terminate inside a crate-private `yo-tui::terminal::backend`
adapter. Nix and Signal Hook types MUST terminate inside a private
`yo-cli::process::termination` adapter. The workspace MUST deny unsafe Rust by
default. Only that process adapter's isolated disposition module MAY locally
allow unsafe code solely to pass a Nix-constructed action or the exact prior
action returned for the same signal to Nix `sigaction`. Deterministic lifecycle
tests MUST use recording fakes rather than a live terminal.

Acceptance MUST include macOS and Linux compilation, deterministic backend and
partial-failure tests, process-coordinator state-model and subprocess signal
tests covering both finalization CAS outcomes, the panic cutoff, concurrent
signal selection, the handler race with `ACTIVE -> CLEANING`, pending-bit
preservation, compile-time `!Send` enforcement, same-thread mask restoration,
process-lifetime handler storage, idle override, every injected installation
and shutdown failure point, and every `Drop` phase; unsafe-scope enforcement;
and
environmental evidence for a local modern terminal, tmux, an SSH PTY, and tmux
inside that SSH session.
Environment-unavailable matrix entries MUST remain explicit rather than being
reported as deterministic failures.

## Rationale

Crossterm alone has the desired mature event decoder, but its raw-mode helper
stores the original termios state only after the mutation reports success, which
cannot satisfy the pre-registered compensation contract for an uncertain
partial failure. Rustix supplies the narrow, I/O-safe termios operations needed
for exact capture and restoration without requiring a custom escape-sequence
parser. Nix provides the narrow exact-disposition boundary unavailable from
Signal Hook's registry, while Signal Hook supplies same-signal default emulation
without terminal writes inside the asynchronous handler.

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
