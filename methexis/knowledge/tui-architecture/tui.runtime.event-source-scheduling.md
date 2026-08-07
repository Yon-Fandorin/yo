---
schema: methexis.knowledge/v1alpha1
id: tui.runtime.event-source-scheduling
kind: decision
owner: tui-architecture
sources:
  - id: tui.events-001
    revision: sha256:6eee66c1defd59cb5828130d9a7e248f27340bdabf9ff6d6623a82ecf4d421b8
relations:
  depends_on:
    - tui.runtime.frame-scheduling
    - tui.runtime.typed-flow
  constrained_by:
    - tui.dependencies.terminal-backend-selection
    - tui.runtime.process-termination-coordinator
    - tui.terminal.lifecycle-restoration
  applies_to:
    - yo-cli::process::termination
    - yo-core::readiness
    - yo-tui::runner::unix
    - yo-tui::terminal::backend::unix::event
---
# Fair readiness-driven event-source scheduling

## Statement

The live frontend MUST treat terminal input, agent events, workspace-reference
events, and skill-reference events as four ordinary event sources. It MUST keep
a deterministic rotating cursor over the live ordinary sources. Starting at
that cursor, each selection MUST inspect sources in cyclic order and choose the
first ready source. Once it handles one ordinary observation, it MUST advance
the cursor to that source's successor and select again before handling another
observation. Any continuously ready ordinary source therefore MUST NOT wait
behind more than one handled observation from each of the other live ordinary
sources. This bound is symmetric across terminal, agent, workspace, and skill.

Process termination MUST remain outside that rotation as a strict-priority
control path. The terminal owner MUST check it before selecting an ordinary
source and again after polling any ordinary source but before applying that
source's result. When termination is observed at either check, it MUST win and
the ordinary result MUST NOT be applied. This includes the check immediately
after terminal input polling and preserves the existing cleanup and same-signal
replay contract. Suspend, user exit, and semantic input priority remain owned by
their existing contracts.

Every live ordinary source and the process-termination source MUST expose
readiness that can register the terminal owner's waker. Buffered work MUST
remain level-ready across bounded or one-item consumption; notification
coalescing MUST NOT strand a non-empty queue. Disconnect and terminal input
failure MUST remain observable outcomes rather than silent loss.

When no source is ready, the owner MUST register interest with every live source
before waiting. With no frame deadline, motion deadline, or active backpressure
retry, that wait MUST be indefinite: no fixed input, termination, or custom
provider polling fallback is permitted. The 10 millisecond worker retry MAY be
armed only while an agent control or dispatch operation is actively
backpressured. A wake or handled observation MAY request a frame but MUST NOT
bypass `tui.runtime.frame-scheduling`.

Acceptance MUST deterministically prove first-ready cyclic selection, the
one-observation bound symmetrically for all four ordinary sources under
simultaneous and continuous readiness, strict termination precedence after
polling each kind of ordinary source and before applying its result, level
readiness across queued work, wake registration without lost notifications,
observable source disconnect and terminal-input failure, indefinite idle
waiting, and the confinement of timed retry to active backpressure.

## Rationale

Bulk-draining one source makes latency depend on unrelated queue depth even when
all producers are individually bounded. Rotating after one observation bounds
cross-source interference without introducing threads, priorities that can
starve background state, or periodic wakeups. The semantic policy is independent
of Crossterm and can be reused by a future GUI while that frontend retains its
native event-loop and rendering adapters.
