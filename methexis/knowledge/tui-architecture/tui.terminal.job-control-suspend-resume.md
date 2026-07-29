---
schema: methexis.knowledge/v1alpha1
id: tui.terminal.job-control-suspend-resume
kind: decision
owner: tui-architecture
sources:
  - id: tui.terminal-003
    revision: sha256:a2b88dbf64fa1780746fc6afd1fbe5137b1569edc0d8562d3dfdce723e3babc2
relations:
  depends_on:
    - tui.terminal.lifecycle-restoration
  constrained_by:
    - tui.architecture.module-boundaries
    - tui.runtime.process-termination-coordinator
  validated_by:
    - terminal.job-control-state-tests
    - terminal.job-control-subprocess-tests
  applies_to:
    - yo-cli::process::job-control
    - yo-tui::runner
---
# Job-control suspension and resumption

## Statement

`Ctrl+Z` during an active terminal session MUST request job-control suspension,
not a normal or termination exit. The terminal-owning thread MUST stop terminal
event production and attempt the complete explicit restoration path before the
process host performs the operating system's default suspend action. Cleanup
failures MUST remain reportable and MUST NOT skip later cleanup attempts.

The process host MUST own the default suspend action and continuation
observation. It MUST NOT implement suspension as a numeric exit code, leave the
process stopped under a custom handler, or expose Unix signal identity through
the frontend-independent application state. Job-control handling MUST remain
separate from the process termination coordinator's termination priority and
same-signal replay contract.

Completing restore-before-stop MUST close the process host's current active
cleanup lease. If a configured termination signal is selected when that lease
finalizes, termination MUST win: the host MUST skip the default suspend action
and replay the selected signal under its existing termination contract. If no
termination is selected, the host MAY perform the default suspend action only
after the coordinator reaches its idle phase.

After continuation, the same terminal-owning thread MUST transactionally
open a fresh active cleanup lease and reacquire the previously selected Inline
or Fullscreen presenter inside it. Application, Session, active Turn,
transcript, editor, pending request, and scroll state MUST live outside a
terminal lease and survive suspension. The first resumed frame MUST distrust
all pre-suspend terminal contents and perform a full redraw. Inline MUST
establish new viewport ownership; Fullscreen MUST reacquire alternate-screen
ownership.

If reacquisition fails or panics after a partial mutation, every registered
compensation MUST be attempted and the live session MUST return a structured
failure rather than continuing with uncertain terminal ownership. Repeated
suspend and resume cycles MUST preserve these guarantees without accumulating
handlers, terminal modes, or presenter state.

## Rationale

Shell job control temporarily transfers terminal ownership without ending the
agent session. Restoring before the stop keeps the shell and other foreground
jobs usable, while transactional same-mode reacquisition and a mandatory full
redraw avoid trusting terminal contents that may have changed while `yo` was
suspended.
