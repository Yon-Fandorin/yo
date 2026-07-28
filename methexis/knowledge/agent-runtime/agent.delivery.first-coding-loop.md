---
schema: methexis.knowledge/v1alpha1
id: agent.delivery.first-coding-loop
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-006
    revision: sha256:5f242f01d7c50a1cffdb032b44fa0e9bf64eba92f88f60daefb3d60c61f8465b
relations:
  depends_on:
    - agent.backend.codex-app-server
    - agent.runtime.active-turn-input
  constrained_by:
    - tui.terminal.lifecycle-restoration
---
# First executable coding loop

## Statement

The first executable agent milestone MUST connect app-server startup and
initialization, creation of one new Session, prompt submission, streamed agent
text, one completed tool Activity and file-change observation, approval request
and response, Turn completion or interruption, explicit failure reporting, and
child-process plus terminal cleanup through `yo-cli`, `yo-core`, and `yo-tui`.

The milestone MUST provide deterministic happy, approval, interruption, and
failure paths through the fake backend, including completed tool and file-change
events. An environment-dependent integration path for a compatible local Codex
installation MUST complete a real tool action and verify its observable file
change in a disposable workspace. A missing Codex binary, initialization or
Session failure, unsupported or malformed protocol input, unexpected child
exit, Turn failure, and cleanup failure MUST remain distinguishable.

Existing Session resume or listing, fork, archive, rollback, queued input,
WebSocket or remote transport, multiple active Sessions, another backend, and a
GUI are out of scope.

## Rationale

This is the smallest vertical slice that proves yo is a coding-agent interface
rather than a chat-only rendering demo, while leaving history, distribution,
and multi-provider expansion for evidence-backed follow-up work.
