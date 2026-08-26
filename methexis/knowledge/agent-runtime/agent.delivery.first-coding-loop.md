---
schema: methexis.knowledge/v1alpha1
id: agent.delivery.first-coding-loop
kind: decision
owner: agent-runtime
sources:
  - id: agent.runtime-006
    revision: sha256:cc559a3c673fbcfe942013c506573a7552a270494328f155a9a65e3d5943e330
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

Existing Session listing, fork, archive, rollback, queued input, WebSocket or remote transport, multiple active Sessions, another backend, and a GUI remain outside that first milestone.

The executable agent path MUST retain the existing interactive TUI as the default `yo` invocation and MUST additionally expose `-p` and `--print` as equivalent non-interactive one-shot spellings. Print mode MUST use the same startup target, model, reasoning, replay, local-tool, persistence, usage, cache, and process-cleanup contracts used by the corresponding interactive Session path. Without `--resume`, it MUST create one ordinary durable new Session. With `--resume SESSION_ID`, it MUST recover that exact stored Session and append one ordinary Turn. It MUST preserve the previously durable Session identity, Provider/Account/Model binding, tool-registry revision, replay lineage, usage accounting, cache accounting, and request lineage. The new Turn MAY append ordinary Session, Journal, replay, usage, and cache records; it MUST NOT reset, replace, rewrite, or otherwise mutate the previously durable state. Print mode MUST NOT itself change tool exposure, grant an approval, or promise one Provider request.

Print resume MUST require an explicit Session ID and MUST reject `--continue`; it MUST NOT infer the newest Session. It MUST reject `--model`, the creation-only `--no-tools` restriction, `--inline`, `--fullscreen`, and `--ascii` when `--resume` is selected. If the selected Session lacks a newest durable Continuation Anchor, has no executable binding under its recorded continuation strategy, or would require a lossy handoff, print resume MUST fail closed before Backend dispatch. It MUST produce no stdout, create no fresh Session, open no replacement binding, and request no interactive confirmation. Every recovery, binding, credential, workspace, or replay failure MUST likewise fail closed without switching Provider or Model, retrying, steering, or falling back.

One print invocation MUST accept one non-empty UTF-8 user input from a positional prompt, piped stdin, or both. When both carry bytes, stdin precedes the positional prompt; the host inserts one LF only when the stdin bytes do not already end in LF. A TTY stdin with no positional prompt and an empty non-TTY input MUST fail before Session or Backend creation. After startup or exact recovery, print mode MUST dispatch exactly one immutable user Submission and wait for its admission and terminal Turn outcome. Internal tool rounds and Provider requests remain owned by the selected Backend. A request that requires interactive approval or user input MUST fail closed without granting or inventing a response.

The resumed print result MUST include only the last completed AgentMessage of the Turn created by the resumed Submission. Every message, segment, Transcript observation, or Live Projection state recovered before that Submission MUST be excluded from stdout. This remains true after snapshot recovery, message normalization, context-compaction handoff, or any other durable-prefix transformation. On a completed Turn, print mode MUST preserve the selected AgentMessage's UTF-8 bytes, appending one LF only when the message does not already end in LF. It MUST withhold output until Session and ordinary process-coordinator cleanup have succeeded, then write that framed message to stdout and exit successfully. Thinking, Working, tool calls, tool results, file changes, usage, cache accounting, Session identity, request traces, prior Turn content, and progress MUST NOT appear in default stdout. Startup, recovery, submission rejection, Turn failure or interruption, an interactive response requirement, a completed Turn without a completed AgentMessage, or cleanup failure MUST produce no stdout and MUST remain a distinguishable stderr failure with a non-success exit. A stdout write failure MUST remain distinguishable even though an operating-system write can already have transferred a prefix. Print mode MUST acquire no terminal presentation state. Process termination MUST use the existing coordinated agent cleanup before applying the selected signal disposition.

Print mode MUST have deterministic evidence for argument and stdin composition, new and resumed selection, exactly one Submission, prior-history exclusion, hidden non-final Activities, completed final-message output, and each discriminating no-output failure boundary. Environment-dependent checks remain explicit rather than being treated as deterministic coverage.

Structured or streaming output and default emission of usage, cache, or Session metadata remain out of scope.

## Rationale

The first vertical slice proves that yo is a coding-agent interface rather than a chat-only rendering demo. The default print contract follows established one-shot coding-agent CLIs: a shell pipeline receives one clean final answer while interactive progress and machine metadata require separate explicit surfaces. Reusing the ordinary Session and Backend semantics keeps `-p` a frontend choice instead of a second agent engine, while exact recovery and closed failure behavior prevent non-interactive execution from silently approving work, selecting a different Session, or exposing ineligible partial results.
