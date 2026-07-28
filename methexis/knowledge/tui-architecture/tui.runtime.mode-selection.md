---
schema: methexis.knowledge/v1alpha1
id: tui.runtime.mode-selection
kind: decision
owner: tui-architecture
sources:
  - id: tui.mode-001
    revision: sha256:241f742ee7db493a368c0a70db766f900a1517bb623d072726fe9d9de50f02d2
relations:
  depends_on:
    - tui.terminal.lifecycle-restoration
  constrained_by:
    - tui.architecture.module-boundaries
  validated_by:
    - cli.mode-selection-tests
    - terminal.fullscreen-frame-tests
    - terminal.fullscreen-restoration-tests
  applies_to:
    - yo-cli::mode-selection
    - yo-tui::runner
---
# Provisional live TUI mode selection

## Statement

The live `yo` CLI MUST expose explicit Inline and Fullscreen selections. Both
selections MUST drive the same application state and interaction outcomes,
while their presenters retain their distinct terminal-ownership and
restoration contracts.

Until an Auto algorithm is separately approved, invoking `yo` without a mode
option MUST preserve the currently deployed Inline behavior. An explicit
Inline or Fullscreen selection MUST override that compatibility default before
terminal state is acquired. The implementation MUST NOT publish an unreviewed
environment heuristic as Auto behavior.

The no-option behavior is provisional rather than the long-term product
default. Choosing whether Inline, Fullscreen, or Auto becomes the eventual
default requires a separate decision informed by comparable agent tools and
the supported terminal, tmux, SSH, and remote-tmux validation matrix.

## Rationale

An explicit Fullscreen path makes both presenters usable and testable now
without changing existing invocations or pretending that the repository has
enough evidence for a permanent default. Keeping selection before terminal
acquisition avoids partially entering one mode and falling back to another.
