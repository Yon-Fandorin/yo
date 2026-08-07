---
schema: methexis.knowledge/v1alpha1
id: tui.runtime.frame-scheduling
kind: decision
owner: tui-architecture
sources:
  - id: tui.frame-001
    revision: sha256:400f3aaa51fb9b8e9ddae487143b5e55adc1dc0465222aca264445568d7df28e
relations:
  depends_on:
    - tui.runtime.typed-flow
  constrained_by:
    - tui.runtime.activity-motion-scheduling
    - tui.terminal.lifecycle-restoration
  applies_to:
    - yo-cli::config::tui.max_fps
    - yo-tui::runner::frame
    - yo-tui::runner::unix
    - yo-tui::TuiSession::with_frame_rate_limit
---
# Bounded live frame scheduling

## Statement

Each live terminal ownership generation MUST create one runner-owned frame
scheduler. Semantic state transitions MUST request a frame through that
scheduler rather than presenting independently. Ordinary terminal input, agent,
workspace, skill, and due motion changes MUST coalesce at one common frame
boundary.

The default ordinary coalescing cadence MUST be 120 frames per second. A host
MAY select 60 frames per second instead; 60 and 120 are the only supported
values in this revision. After a completed ordinary frame, the next coalesced
frame MUST wait until at least `1_000_000_000 / fps` nanoseconds after that
completion. The first visible frame of a terminal generation and a frame
requested by terminal resize are explicit correctness exceptions: they MUST be
immediate and MUST NOT wait for the ordinary coalescing boundary. The selected
cadence therefore bounds coalesced ordinary presentation, not those two
immediate cases. Zero-sized terminal geometry MUST suppress presentation work.

The CLI configuration key `tui.max_fps` MUST accept numeric `60` or `120`, MUST
default to `120`, and MUST reject every other value explicitly. The key selects
the ordinary coalescing cadence despite its historical `max_fps` name; the
immediate first-frame and resize exceptions remain mandatory. Live startup MUST
read this value once before opening the repeatable terminal-generation loop.
The selected value MUST survive suspend and resume generations for that live
session. Runtime configuration reload is outside this revision.

A due motion deadline MUST become a retained coalesced frame request before its
deadline is cleared. Waiting MUST then target the frame boundary rather than the
already-consumed motion deadline, so a 60fps selection cannot create a
zero-timeout loop around a faster motion period.

Acceptance MUST prove the default and selected ordinary intervals, ordinary
request coalescing, the explicit immediate bypass for first and resize frames,
due-motion retention, zero-size suppression, configuration validation, and
retention of the startup selection across terminal ownership generations.

## Rationale

One request scheduler separates responsiveness policy from state transitions
and prevents several ready sources from causing redundant terminal writes.
Immediate first and resize frames restore trustworthy geometry, while ordinary
updates remain bounded for local and remote terminals. The policy is not tied
to Crossterm; a future GUI may reuse the coalescing semantics while mapping
presentation to its native event loop or display synchronization boundary.
