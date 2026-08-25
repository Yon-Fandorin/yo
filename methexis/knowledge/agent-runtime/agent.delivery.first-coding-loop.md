---
schema: methexis.knowledge/v1alpha1
id: agent.delivery.first-coding-loop
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-006
    revision: sha256:0a8ce7e52b0b85a7985f39a38223a33fd53de5aef268e8b0850de9ef19e3d9b3
relations:
  depends_on:
    - agent.backend.codex-app-server
    - agent.runtime.active-turn-input
  constrained_by:
    - tui.terminal.lifecycle-restoration
---
# Executable coding delivery

## Statement

The first executable agent milestone MUST connect app-server startup and initialization, creation of one new Session, prompt submission, streamed agent text, one completed tool Activity and file-change observation, approval request and response, Turn completion or interruption, explicit failure reporting, and child-process plus terminal cleanup through `yo-cli`, `yo-core`, and `yo-tui`.

The milestone MUST provide deterministic happy, approval, interruption, and failure paths through the fake backend, including completed tool and file-change events. An environment-dependent integration path for a compatible local Codex installation MUST complete a real tool action and verify its observable file change in a disposable workspace. A missing Codex binary, initialization or Session failure, unsupported or malformed protocol input, unexpected child exit, Turn failure, and cleanup failure MUST remain distinguishable.

Existing Session resume or listing, fork, archive, rollback, queued input, WebSocket or remote transport, multiple active Sessions, another backend, and a GUI remain outside that first milestone.

The executable agent path MUST retain the existing interactive TUI as the default `yo` invocation and MUST additionally expose `-p` and `--print` as equivalent non-interactive one-shot spellings. Print mode MUST create one ordinary durable new Session through the same startup target, model, reasoning, replay, local-tool, persistence, usage, cache, and process-cleanup contracts used by an interactive new Session. `--model` and the creation-only `--no-tools` restriction remain independent startup choices; print mode MUST NOT itself change tool exposure, grant an approval, or promise one Provider request. Initial print mode MUST reject `--resume`, `--continue`, `--inline`, `--fullscreen`, and `--ascii` rather than assigning unreviewed continuation or terminal-presentation meaning.

One print invocation MUST accept one non-empty UTF-8 user input from a positional prompt, piped stdin, or both. When both carry bytes, stdin precedes the positional prompt; the host inserts one LF only when the stdin bytes do not already end in LF. A TTY stdin with no positional prompt and an empty non-TTY input MUST fail before Session or Backend creation. After startup, print mode MUST dispatch exactly one immutable user Submission and wait for its admission and terminal Turn outcome. Internal tool rounds and Provider requests remain owned by the selected Backend. A request that requires interactive approval or user input MUST fail closed without granting or inventing a response.

On a completed Turn, print mode MUST reconstruct the last completed AgentMessage for that Turn and preserve its UTF-8 bytes, appending one LF only when the message does not already end in LF. It MUST withhold output until Session and ordinary process-coordinator cleanup have succeeded, then write that framed message to stdout and exit successfully. Thinking, Working, tool calls, tool results, file changes, usage, cache accounting, Session identity, request traces, and progress MUST NOT appear in default stdout. Startup or submission rejection, Turn failure or interruption, an interactive response requirement, a completed Turn without a completed AgentMessage, or cleanup failure MUST produce no stdout and MUST remain a distinguishable stderr failure with a non-success exit. A stdout write failure MUST remain distinguishable even though an operating-system write can already have transferred a prefix. Print mode MUST acquire no terminal presentation state. Process termination MUST use the existing coordinated agent cleanup before applying the selected signal disposition.

Print mode MUST have deterministic evidence for argument and stdin composition, exactly one Submission, hidden non-final Activities, completed final-message output, and each discriminating no-output failure boundary. Environment-dependent checks remain explicit rather than being treated as deterministic coverage.

Print continuation, structured or streaming output, and default emission of usage, cache, or Session metadata remain out of scope.

## Rationale

The first vertical slice proves that yo is a coding-agent interface rather than a chat-only rendering demo. The default print contract follows established one-shot coding-agent CLIs: a shell pipeline receives one clean final answer while interactive progress and machine metadata require separate explicit surfaces. Reusing the ordinary Session and Backend semantics keeps `-p` a frontend choice instead of a second agent engine, while the closed failure behavior prevents non-interactive execution from silently approving work or exposing ineligible partial results.
