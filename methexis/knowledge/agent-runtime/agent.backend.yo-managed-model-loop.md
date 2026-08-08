---
schema: methexis.knowledge/v1alpha1
id: agent.backend.yo-managed-model-loop
kind: decision
owner: agent-runtime
sources:
  - id: agent.backend-008
    revision: sha256:e217dbe1738d7d6d8fb1e2d761cee5ccf07795a2d1b1801a379447e75adec2dd
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.connector.openai-chat-completions
    - agent.connector.openai-responses
    - agent.observability.session-journal
    - agent.runtime.command-event-boundary
    - agent.runtime.session-turn-activity
    - agent.session.continuation-lineage
    - agent.tool.local-execution-boundary
---
# Yo-managed model and tool loop

## Statement

The Yo-managed Agent Backend MUST implement the existing `AgentBackend` semantic port while owning the model loop, tool execution coordination, and model-visible context inside `yo-core`. The effective binding MUST select one admitted Model Connector and exact API dialect before a Turn starts. A Model Connector MUST own only remote request and stream protocol and MUST translate its dialect into the connector-neutral round observations consumed by the loop. Neither `yo-cli`, a frontend, nor the connector may become the agent-loop owner. The loop MUST NOT probe another dialect, fall back to another connector, or branch on Provider identity.

For each accepted Turn, the backend MUST project the committed semantic Session history plus the new user input into the selected API dialect. Text deltas MUST become `ModelWork` Activities through the existing message segmentation and terminal-seal path. A model function call MUST preserve its wire call identity, function name, and exact accumulated argument bytes. It MUST become a correlated Tool Activity even when validation rejects it; invalid JSON, schema mismatch, unknown or duplicate identity, unavailable tool, and argument-bound failure MUST terminate that Activity with the typed validation failure and no effect. Validation MUST succeed before approval, admission, or dispatch. Approval and execution MUST use the frozen registry, admission policy, and execution-host boundary; the model service MUST NOT directly execute local workspace tools.

The backend MUST record each function call and its exact tool outcome before submitting a corresponding function-call output through the selected dialect in the next model request. Multiple calls returned by one response MAY execute concurrently only when the tool scheduler proves their approval and mutable resource leases independent; otherwise they MUST execute in model order. Results MUST be returned in stable call order regardless of execution completion order. A missing, duplicate, or mis-correlated call or result MUST fail the Turn.

The loop continues across model response, local tool execution, and tool-result submission until the model emits a final assistant message, cancellation is accepted, a bounded model-round limit is reached, or a typed failure occurs. One active Turn remains the Session limit. Cancellation MUST stop outstanding connector work promptly, prevent new tool execution, seal active Activities as interrupted, and run explicit connector and tool cleanup.

Provider response IDs, cache handles, and conversation IDs MAY be retained as diagnostic correlation but MUST NOT be the only continuation locator. A Yo-managed binding MUST explicitly declare `exact_replay` with `local_client` as the current executor rather than provider-native resume. Executable continuation MUST reconstruct the model-visible semantic boundary named by the newest durable Continuation Anchor from the Session Journal and open a new binding epoch when endpoint, API dialect, Provider, Account, Model, or connector identity changes. A committed mid-Turn function call, tool result, partial stream, or other suffix beyond that Anchor MUST remain diagnostic and MUST NOT become automatic continuation input. When no durable Anchor exists, the Session MUST follow the continuation contract's read-only fallback rather than constructing replay input. Exact replay MUST preserve message roles and order, exact visible text, function-call and tool-result relationships, and the recorded system/tool contract. Hidden reasoning and provider cache state are not replay claims.

No Continuation Anchor may cover a partial model stream, an uncommitted tool result, an uncertain request, or a failed final response. Usage and the exact effective binding MUST be attributed to the model response that produced them, including when the model changes inside one Yo Session.

The selected model catalog entry MUST provide an input-token limit, an output reserve, and the exact tokenizer profile used by an injected token counter. Every model request MUST pass that counter before dispatch. Provider-side implicit caching MAY reduce billing but MUST NOT change exact replay or context admission. When exact replay no longer fits, the backend MUST return typed `context_exhausted`, complete the current Turn as non-resumable, and reject a later Turn on that binding. It MUST NOT silently discard, truncate, or summarize history. Lossy compaction is deferred to an independently reviewed, user-visible handoff that opens a new binding epoch.

Tool arguments and outputs MUST pass the local tool boundary's semantic-admission gate before they become Activities, later model input, or a replay delta. The backend MUST persist replay as its own semantic record and MUST NOT attach it to the payload-free resumable-outcome correlation record.

A future `managed_server` executor MAY load the same validated replay prefix and assemble the next model request on a Yo-managed Session service. It does not define a second replay meaning and MUST use the same replay contract, ordering, bounds, and Anchor boundary as `local_client`. It remains deferred until its remote repository, identity, digest, availability, and retention evidence has an independently reviewed implementation. The current backend MUST NOT advertise it.

## Rationale

Owning the loop in `yo-core` provides a genuinely native backend while preserving the existing frontend-independent Session contract. An explicit connector boundary lets multiple API dialects share the semantic loop without weakening their distinct wire grammars. Exact semantic replay avoids coupling durable continuation to a provider's temporary response retention and keeps tool side effects correlated with Yo's own authority.
