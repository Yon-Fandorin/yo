---
schema: methexis.knowledge/v1alpha1
id: tui.dependencies.terminal-backend-selection
kind: decision
owner: tui-architecture
sources:
  - id: tui.dependencies-001
    revision: sha256:e73ad00259d797bfaaabf7fd44eb806d0aabc45d76a3bc0f92b3e4fb18a8e9a1
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

- `crossterm = 0.29.0` with default features disabled and only `events`,
  `bracketed-paste`, plus `event-stream` enabled, for owner-thread readiness and
  decoding of key, paste, focus, mouse, and resize events;
- `futures-core = 0.3.33`, only to poll Crossterm's `EventStream` through its
  crate-private `Stream` boundary without introducing an async runtime or
  executor;
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
enable Crossterm's `serde`, `windows`, `derive-more`, `osc52`, or `use-dev-tty`
features, Rustix's PTY or unrelated syscall features, Nix features other than
`signal`, or Signal Hook's iterator and extended signal-information features.
`signal-hook` remains on the latest `0.3` patch to share Crossterm's compatible
dependency graph; moving to `0.4` requires a separate compatibility review.

The terminal-owning thread MUST poll `EventStream` itself and translate it into
public yo input values. This dependency choice MUST NOT introduce another
terminal owner, async executor, or periodic input-polling fallback.

Yo MUST retain ownership of `Surface`, `FrameDiff`, `TerminalOp`, ANSI output,
mode acquisition order, cleanup policy, and public input events. Crossterm,
Futures Core, and Rustix types MUST terminate inside a crate-private
`yo-tui::terminal::backend` adapter. Nix and Signal Hook types MUST terminate
inside a private `yo-cli::process::termination` adapter. The workspace MUST deny
unsafe Rust by default. Only that process adapter's isolated disposition module
MAY locally allow unsafe code solely to pass a Nix-constructed action or the
exact prior action returned for the same signal to Nix `sigaction`.
Deterministic lifecycle tests MUST use recording fakes rather than a live
terminal.

Acceptance MUST include macOS and Linux compilation, deterministic backend and
partial-failure tests, owner-thread readiness tests proving that terminal input
wakes an indefinite wait without a periodic fallback, process-coordinator
state-model and subprocess signal tests covering both finalization CAS outcomes,
the panic cutoff, concurrent signal selection, the handler race with
`ACTIVE -> CLEANING`, pending-bit preservation, compile-time `!Send`
enforcement, same-thread mask restoration, process-lifetime handler storage,
idle override, every injected installation and shutdown failure point, and every
`Drop` phase; unsafe-scope enforcement; and environmental evidence for a local
modern terminal, tmux, an SSH PTY, and tmux inside that SSH session.
Environment-unavailable matrix entries MUST remain explicit rather than being
reported as deterministic failures.

## Rationale

Crossterm's event decoder remains the narrowest mature input boundary. Its
`EventStream` supplies readiness to the existing terminal owner, eliminating a
fixed input-poll interval without transferring event-loop or rendering ownership
to Crossterm or an async runtime. `futures-core` exposes only the polling trait
needed for that boundary.

Crossterm's raw-mode helper stores the original termios state only after the
mutation reports success, which cannot satisfy the pre-registered compensation
contract for an uncertain partial failure. Rustix supplies the narrow, I/O-safe
termios operations needed for exact capture and restoration without requiring a
custom escape-sequence parser. Nix provides the narrow exact-disposition
boundary unavailable from Signal Hook's registry, while Signal Hook supplies
same-signal default emulation without terminal writes inside the asynchronous
handler.

A Rustix-only adapter would require yo to build and maintain modern keyboard,
paste, mouse, focus, and resize parsing. Termwiz duplicates yo's existing
Surface, cell, style, and renderer ownership and documents a comparatively
unstable API. Termina is the closest modern replacement candidate because its
lower-level parser keeps terminal protocols visible, but its still-young pre-1.0
adapter surface has less accumulated compatibility evidence than Crossterm and
also exposes escape, style, and terminal ownership that yo does not need. The
selected split keeps those larger policy surfaces out of the dependency boundary
while leaving each library replaceable behind typed yo values. Termina SHOULD be
reconsidered through a separate compatibility review if the initial environment
matrix exposes a Crossterm limitation.
