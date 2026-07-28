---
schema: methexis.knowledge/v1alpha1
id: tui.runtime.process-termination-coordinator
kind: decision
owner: tui-architecture
sources:
  - id: tui.process-001
    revision: sha256:1399ceade012efc95e5d819aa0305716e9c1db8ac0d86f4049594e2310da88df
relations:
  depends_on:
    - tui.runtime.typed-flow
    - tui.terminal.lifecycle-restoration
  constrained_by:
    - tui.architecture.module-boundaries
  validated_by:
    - process.termination-state-model
    - process.termination-subprocess-matrix
    - process.termination-failure-injection
  applies_to:
    - yo-cli::process::termination
---
# Process termination coordinator

## Statement

The private `yo-cli` process host MUST own exactly one Unix termination
coordinator. The coordinator MUST be thread-bound and `!Send`; initialization,
failed-install rollback, every active-session lease, explicit shutdown, and
`Drop` MUST execute on the installing thread so caller-mask restoration cannot
target a different thread. Its lifecycle is irreversible:

```text
NEW -> INSTALLING -> IDLE
                     |
                     v
                   ACTIVE -> CLEANING -> IDLE
                                 |
                                 v
                            TERMINATING

IDLE -> SHUTTING_DOWN -> RETIRED
INSTALLING or SHUTTING_DOWN failure -> FAILED_RETIRED
```

`RETIRED` and `FAILED_RETIRED` MUST never transition to a live state. Once any
handler installation might have become observable, failed initialization also
retires the process coordinator. It MUST attempt every applicable installation
rollback in reverse order, preserve the primary and all rollback failures, and
MAY restore captured prior actions and masks as part of that exact failed-install
rollback. Once any installed handler might have become observable, its reachable
static storage MUST remain valid for the rest of the process even when rollback
or later shutdown succeeds.

One packed lock-free atomic word MUST contain the coordinator phase and pending
bits for `SIGHUP`, `SIGINT`, `SIGQUIT`, and `SIGTERM`. Handler publication and
session finalization MUST use compare-exchange on that same word as their single
linearization point. Every transition out of a handler-observable publishing
phase, including `ACTIVE -> CLEANING`, MUST use a compare-exchange loop that
preserves pending bits published concurrently. A handler that observes `ACTIVE`
or `CLEANING` publishes its signal bit. If cleanup reaches `IDLE` first, the
handler reloads that state and immediately invokes the selected signal's default
disposition. Selection among bits successfully published before the session
finalization CAS MUST use the stable priority `SIGHUP`, `SIGINT`, `SIGQUIT`,
then `SIGTERM`, and replay the selected signal unchanged. That successful CAS
is the selection cutoff. A later handler observes the resulting `IDLE` or
`TERMINATING` phase and takes the fail-closed default path rather than joining
the earlier snapshot.

Every handler-observable phase MUST be closed. `ACTIVE` and `CLEANING` publish
through the packed word. `IDLE`, `INSTALLING`, `SHUTTING_DOWN`, `RETIRED`,
`FAILED_RETIRED`, and `TERMINATING` MUST immediately invoke the received
signal's default disposition. `NEW` is not handler-observable. Static storage
reachable by a delayed handler MUST remain valid in every phase.

The host MUST lend at most one active cleanup lease through a closure boundary:

```text
host.with_active_session(|termination_events| yo_tui_session(termination_events))
```

The lease covers `ACTIVE`, terminal cleanup, and final disposition. It MUST
prevent explicit host shutdown while a cleanup obligation exists. `yo-tui`
consumes only typed termination observations and returns only after viewport and
TTY restoration. If a termination bit linearizes before post-cleanup
finalization, signal termination wins over a concurrent panic; diagnostics and
cleanup failures are emitted before same-signal default replay. Otherwise the
original panic resumes.

While the coordinator is `IDLE`, configured signals MUST immediately take their
default action. This intentionally overrides prior custom handlers and
`SIG_IGN` for the coordinator's lifetime. Exact prior actions, flags, signal
masks, and the installing thread's caller mask from a successfully initialized
coordinator MUST be restored only by successful explicit `shutdown()` from
`IDLE`. Exact failed-install rollback is the only exception. Partial restoration
MUST attempt every compensation, return the complete failure report, retain
process-lifetime handler storage, enter `FAILED_RETIRED`, and permanently
reject reinitialization.

`Drop` MUST be non-panicking and fail closed. From `IDLE` it MUST attempt the
explicit shutdown sequence best effort. Successful restoration ends the
coordinator's ownership of OS dispositions and masks, but handler-reachable
storage remains valid for process lifetime because replacing a disposition does
not prove an already executing handler is quiescent. Any failure enters
`FAILED_RETIRED`. From `ACTIVE`, `CLEANING`, or any partial-failure state it MUST
NOT restore dispositions early. Production CLI termination MAY intentionally
retain the coordinator until process exit; explicit shutdown remains the
reportable embedding and test boundary.

Acceptance MUST model every state and transition, both outcomes of handler
publication racing `ACTIVE -> CLEANING` and finalization CAS, preservation of
pending bits across every publishing-phase transition, the signal-versus-panic
cutoff, the concurrent-bit selection cutoff, idle override of custom and
ignored actions, failure injection at every installation and shutdown step,
compile-time rejection of cross-thread coordinator transfer, same-thread
caller-mask restoration, process-lifetime storage after successful rollback
and shutdown, and `Drop` from `IDLE`, `ACTIVE`, `CLEANING`, and partial-failure
states. Subprocess evidence MUST prove cleanup precedes active-session
termination and that exact prior actions and the caller mask return only after
successful shutdown or an exact failed-install rollback.

## Rationale

Unix signal dispositions are process-wide while masks and terminal cleanup are
thread-affine. Keeping one irreversible coordinator above `yo-tui` prevents
session generations from replacing one another and lets repeatable UI sessions
share one termination policy without making a UI library the process lifecycle
root. Packing phase and signal bits into one atomic closes the gap in which
separate state and pending atomics could strand a late publication after
cleanup.
