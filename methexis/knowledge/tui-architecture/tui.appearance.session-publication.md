---
schema: methexis.knowledge/v1alpha1
id: tui.appearance.session-publication
kind: decision
owner: tui-architecture
sources:
  - id: tui.appearance-001
    revision: sha256:a12559aef8ef3ac565758ae82fcba43e374a86066e9d4154df9aaf57535b03c5
relations:
  constrained_by:
    - tui.architecture.module-boundaries
    - tui.surface.resolved-style
  applies_to:
    - yo-tui::runner::TuiSession
    - yo-tui::appearance
---
# Session-owned appearance publication

## Statement

Each `TuiSession` MUST own exactly one immutable committed appearance snapshot
and a monotonic revision. The snapshot contains the fully resolved styles,
glyphs, and layout configuration needed by the session; physical `Surface`
cells continue to store only final resolved `Style`.

The TUI owner thread MUST be the single writer. It MUST validate and resolve a
complete candidate outside logical frame preparation. An invalid candidate
MUST return an explicit rejection and preserve both the committed snapshot and
its revision. A valid candidate MUST atomically replace the whole snapshot,
advance the revision, and become visible at the next logical frame boundary.
Published fields MUST NOT be mutated individually.

Appearance selection MUST NOT live in process-global mutable state,
thread-local current scope, or hidden string-key lookup. The committed snapshot
and revision MUST survive terminal suspend and resume; generation-local
presenter history may be rebuilt without re-resolving appearance when no new
generation-specific capability input exists.

The initial runtime replacement seam MAY remain crate-private. A public
appearance API requires a separately reviewed host consumer and documentation
contract.

## Rationale

A session-owned, whole-value publication boundary makes live replacement
deterministic, isolates concurrent sessions, and leaves room for future
configuration without moving semantic theme state into `Surface`.
