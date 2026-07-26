---
schema: methexis.knowledge/v1alpha1
id: tui.surface.validation-matrix
kind: decision
owner: tui-architecture
sources:
  - id: tui.surface-011
    revision: sha256:7f049056042e8f724e460d06e1324d673c08374cceebe4542789d21bf542f43b
relations:
  depends_on:
    - tui.surface.html-projection
    - tui.surface.terminal-ops
    - tui.surface.width-profile
  validated_by:
    - terminal.pty-matrix
    - html.surface-parity-fixtures
  applies_to:
    - yo-tui
---
# Rendering validation authority

## Statement

Observed output in a real PTY MUST remain the authority for terminal behavior.
The supported initial matrix MUST cover modern macOS and Linux terminals,
local tmux, SSH, and tmux inside an SSH session. Deterministic HTML fixtures are
diagnostic and parity evidence, not a replacement for terminal checks.

Environment-dependent failures MUST be reported separately from deterministic
model, operation, and projection failures. Unavailable matrix entries MUST
remain explicitly tracked as unverified rather than silently passing.

## Rationale

The matrix covers the environments where agentic TUI work is expected while
keeping environmental uncertainty distinct from reproducible code defects.
