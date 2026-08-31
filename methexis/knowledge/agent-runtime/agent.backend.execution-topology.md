---
schema: methexis.knowledge/v1alpha1
id: agent.backend.execution-topology
kind: decision
owner: agent-runtime
sources:
  - id: agent.backend-007
    revision: sha256:0173528a6b1f486aa92f90b12d2f4b3f76f3ba23ad53248de3a0aa9f9267dc45
relations:
  depends_on:
    - agent.core.frontend-independent-boundary
---
# Agent backend execution topology

## Statement

An Agent Backend MUST be classified independently along orchestration ownership, connector, execution target, transport, workspace-host, and tool-execution-host axes. A Delegated Agent Backend connects to a coding-agent host such as Codex app-server, Grok Build ACP, or Kimi Code; that host owns its agent loop, tool execution, and backend Session. A Yo-managed Backend keeps those responsibilities in yo and uses a Model Connector to reach a service such as OpenAI or Kimi.

`Provider` MUST name a model service rather than a delegated coding-agent process. `Local` and `Remote` MUST describe execution placement, and stdio, SSH, WebSocket, HTTP, and SSE MUST describe transport; neither dimension creates another semantic backend kind. Every Agent Backend MUST report the exact boundary that it can observe for Request diagnostics, through its Connector where one exists, and MUST NOT claim visibility into a downstream request owned by another process or service.

Generic backend lifecycle, capability, failure, evidence, and replay types MUST live in the independent `yo-backend` foundation crate. Its generic replay contract MAY include the smallest bounded versioned opaque provider-private envelope needed for exact durable replay, but MUST NOT interpret a Provider schema or payload. Bounded child-process JSONL, stderr retention, request-ID allocation, and deferred-message mechanisms MAY also be shared there, but host wire interpretation and Yo semantic state MUST NOT enter that foundation. `yo-core` MUST specialize `BackendAdapter` as its provider-neutral `AgentBackend` port and MUST NOT depend on a concrete backend.

Concrete backends MUST remain flat independent crates: `yo-backend-managed`, `yo-backend-delegated-codex`, and `yo-backend-delegated-grok`. Each depends on the foundation and `yo-core` specialization. The process host selects and constructs an admitted adapter. The current local delegated adapters are Codex app-server and Grok Build ACP.

External-review routing MUST represent a managed ModelTarget and a delegated
HostTarget as disjoint first-class target identities. It MUST NOT invent a
ProviderId or AccountId for `host:codex` or `host:grok`. Managed review retains
its exact empty Yo tool registry. Delegated review instead selects
`yo.delegated-review-execution/v1alpha1` through print-mode exact
`--sandbox read-only`; this profile permits only host-owned read-only
inspection and MUST NOT be described as managed `no-tools`.

For that profile, the Codex adapter MUST disable web search, fix
`approvalPolicy` to `never`, and set the sandbox to read-only with tool network
disabled on Thread creation, Thread resume, and every Turn. The Grok adapter
MUST launch ACP with its
read-only sandbox, `dontAsk`, only the `Read` and `Grep` built-ins, web and
subagents disabled, and no Yo-supplied MCP servers. A permission request or
mutable workspace effect outside those bounds is a failed review delivery,
not a prompt for broader authority. The exact profile MUST be frozen in the
durable host binding; resume restores it and MUST NOT silently downgrade to a
normal delegated Session.

One original review sends one immutable packet through one fresh isolated host
Session. Finding-resolution delivery MUST resume that exact reviewer Session
and send one additional immutable packet per step. Frozen authorizations retain
one direct resolution. A separately versioned bounded-multihop authorization
MAY permit an explicit finite maximum; egress MUST count the immutable
review-delta chain and bind every step to the immediately preceding delivery
receipt. The first step beyond the authorized maximum MUST fail before host
delivery. Retry, steer, fallback, target switch, and a second request for the
same step remain forbidden. The delivery claim and receipt MUST name the
HostTarget, execution profile, and one durable host-request identity. They MUST
state the actual host-owned tool boundary, MUST NOT publish managed
`tool_execution: false`, and MUST leave Provider request identity and token
usage unknown unless the delegated host supplies separately reviewed exact
evidence.

The Model Connector boundary MUST remain independent of the Agent Backend boundary. `yo-core` MUST own only the provider-neutral Connector port and shared Connector semantic request, observation, failure, cancellation, and complete-binding types, including the closed registry that derives the exact Connector identity from an admitted `api_dialect` and complete binding. Exact HTTP request construction, dialect stream decoding, endpoint policy, retry grammar, and provider-private payload interpretation MUST NOT enter `yo-core`, `yo-backend`, or `yo-backend-managed`.

Concrete Model Connectors MUST remain flat independent crates under `crates/connectors/`: `yo-connector-openai-responses`, `yo-connector-openai-chat-completions`, and `yo-connector-kimi`. Each MUST depend on `yo-core`, MAY depend on `yo-backend` only for its connector-neutral replay contract and opaque provider-private envelope, MUST implement exactly its admitted Connector identity and dialect, and MUST NOT depend on another concrete Connector. `yo-core`, `yo-backend`, and `yo-backend-managed` MUST NOT depend on a concrete Connector. Kimi request and response grammar, private-assistant schema decoding and codec, lossless validation, extraction of the connector-neutral visible replay projection, and exact encoded-size calculation belong only to `yo-connector-kimi`. It MUST return that validated projection together with the bounded opaque provider-private envelope. `yo-backend` may retain and bound the envelope but MUST NOT interpret its Kimi fields; `yo-backend-managed` may validate only the envelope's declared schema identity, binding epoch, and bounds and compare the Connector-supplied projection with semantic replay.

A flat internal `yo-connector-transport` crate under `crates/connectors/transport` MAY be shared by at least two concrete Connectors only for bounded HTTPS and SSE byte transport, framing, cancellation, cleanup, and delivery mechanics. It MUST NOT own an API dialect, Provider or Model policy, complete binding, semantic replay meaning, retry decision, or provider-private payload interpretation. A concrete Connector MUST remain the sole owner of its request grammar, response terminal, retry admission, and semantic projection.

`yo-core` MUST derive one exact Connector identity through its closed `api_dialect` registry without Provider probing or fallback. `yo-cli`, as the process-wide composition owner, MUST map that already derived exact Connector identity and dialect to one concrete factory and inject it into `yo-backend-managed` and the model-service verification path. That composition MUST NOT probe a Provider, infer a dialect from a Model name, fall back to another Connector, or make the managed loop branch on Provider. The split MUST preserve existing Journal bytes and ordering, binding epochs, replay profiles, visibility exclusions, plaintext-retention consent, request behavior, and terminal behavior without a migration.

## Rationale

Keeping ownership, vendor, placement, and wire protocol orthogonal avoids local-only backend types and lets the same Session semantics cover local Codex and Grok processes, a remote agent host, and a yo-owned model loop. Independent adapter crates keep host protocol churn out of the semantic core and allow a new host without adding another concrete backend dependency to `yo-core`.

Separating delegated read-only review from managed no-tools makes the recorded
evidence honest: Yo can constrain each host through its own reviewed controls,
but it cannot claim ownership of the host's agent loop or downstream Provider
request. Freezing the profile in the binding keeps continuation from widening
permissions, while a common target identity lets exact-once workflow code stay
provider-neutral without manufacturing model-service coordinates.

The three admitted model dialects already change independently and Kimi additionally owns private replay and provider-specific request rules. Flat Connector crates keep that churn out of `yo-core` and the managed loop, while the existing neutral replay foundation retains only correlation, bounds, and opaque durable payload. One narrow transport helper avoids copying byte-lifecycle mechanics without becoming a second semantic owner. Process-root injection preserves exact binding selection and lets terminal or future GUI frontends reuse the same semantic engine without importing every concrete Provider implementation.
