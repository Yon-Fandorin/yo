---
schema: methexis.knowledge/v1alpha1
id: agent.tool.local-execution-boundary
kind: decision
owner: agent-runtime
sources:
  - id: agent.tool-001
    revision: sha256:a840431b5a438967abd623cd018c6a7e1e11508f7f800e0c77f419688e124455
relations:
  depends_on:
    - agent.core.frontend-independent-boundary
    - agent.runtime.command-event-boundary
    - agent.runtime.session-turn-activity
  constrained_by:
    - agent.observability.session-journal
---
# Local model-tool execution boundary

## Statement

A Yo-managed model loop MUST expose only tools admitted through a
frontend-independent registry. Each registered tool MUST have a stable
`ToolId`, a unique wire name, safe description, versioned JSON input schema,
typed effect and approval requirements, and an injected execution-host handle.
The registry and admission policy belong to `yo-core`; the execution host owns
the concrete operating-system or remote workspace effect and MUST NOT expose
that effect to a Model Connector.

The effective tool registry MUST be frozen for one model request. The model
MUST receive only its admitted function-tool projection. Provider built-in
tools, provider-hosted code execution, and direct provider MCP execution are
deferred and MUST NOT be enabled implicitly by an OpenAI-compatible endpoint.

A returned function call MUST resolve one exact registered tool and validate
its complete accumulated JSON arguments before approval or execution. Invalid
JSON, a schema mismatch, an unknown or duplicate call identity, an unavailable
tool, or a request that exceeds configured argument bounds MUST become a typed
Tool Activity failure without dispatching an effect. Approval MUST bind the
exact Turn, call identity, ToolId, normalized argument digest, effect class,
and execution host. A stale or mismatched response MUST NOT authorize a call.

After dispatch, one call permits at most one local execution attempt. Timeout,
transport ambiguity, cancellation, executor failure, or lost output MUST NOT
automatically repeat a potentially effectful tool. The executor MUST return a
typed completed, failed, or interrupted result with bounded textual output and
explicit truncation when applicable. The Session Journal MUST correlate the
exact call, approval, execution attempt, and tool result before that result is
eligible for model submission.

Execution progress and absolute work budgets MUST remain distinct. Every
execution host MUST define a finite progress-inactivity deadline and the exact
signal that resets it. The first local command tool MUST treat each non-empty
stdout or stderr chunk as progress, MUST reset a 5-minute inactivity window on
that progress, and MUST fail the attempt when no such output arrives for 5
minutes even if the process is still alive. Agent policy MAY additionally
supply one absolute execution deadline; it MUST default to absent, begin once
for that attempt, and MUST NOT reset on output or other progress. Cancellation
MUST interrupt both waits, and timeout or cancellation MUST use finite
termination, reap, and output-drain bounds. Diagnostics MUST distinguish
inactivity, an agent-supplied absolute deadline, cancellation, and cleanup
failure. None of these outcomes permits an automatic retry.

Calls MUST execute serially in model order by default. They MAY execute
concurrently only when the scheduler proves that approval scopes and mutable
resource leases are disjoint. Result publication and model submission MUST use
stable model-call order regardless of completion order. Cancellation MUST
prevent undispatched calls, request prompt cancellation of active executors,
and preserve an explicit interrupted result when the host cannot prove that an
effect did not occur.

Tool names, schemas, arguments, and outputs are model-visible semantic history
and MUST follow the Session Journal's bounded persistence and redaction rules.
Execution-host diagnostics and prohibited secrets remain outside semantic
history. Exact replay MUST reproduce the recorded function-call and result
relationship without re-executing the historical tool.


The first registry schema dialect is the closed `yo.tool-schema/v1` subset.
Every node requires one of object, array, string, number, integer, boolean, or
null; only `description`, `properties`, `required`,
`additionalProperties`, `items`, and same-type non-empty `enum` are
admitted. Object schemas MUST set `additionalProperties: false`; arrays require
one item schema; required names MUST be unique declared properties; unsupported
keywords and schema/instance nesting beyond 16 fail closed.

Every validation class MUST expose a stable non-null
`yo.tool.validation.*/v1` failure code separately from diagnostic prose. Before
dispatch, raw validated arguments MUST pass an injected semantic-admission gate.
Tool output MUST pass the same gate before it becomes an Activity, later model
input, or replay: the gate may admit it exactly, replace it with one explicit
bounded redacted value, or fail the Turn. Credentials, complete environment
values, execution-host diagnostics, and configured prohibited literals MUST NOT
cross this boundary, and a concrete tool MUST NOT bypass it. Until a concrete
gate is installed, no local tool registry may be exposed to a native model.

## Rationale

Delegated backends hide tool policy inside another agent host. A native loop
needs an explicit local boundary so model protocol cannot bypass approval,
repeat side effects, or confuse tool completion order with semantic order.
Separating output inactivity from an optional agent-owned absolute deadline
allows productive long commands to continue while still detecting silent
stalls and preserving cancellation.
