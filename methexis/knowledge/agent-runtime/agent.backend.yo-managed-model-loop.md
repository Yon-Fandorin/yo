---
schema: methexis.knowledge/v1alpha1
id: agent.backend.yo-managed-model-loop
kind: decision
owner: agent-runtime
sources:
  - id: agent.backend-008
    revision: sha256:c02627ccf60cfdfa5cfecc8a0ed8adbc839e92b14542f8cbc35a593e45c4b6e3
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.connector.kimi-chat-completions
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

The Yo-managed Agent Backend MUST implement the existing `AgentBackend` semantic port while owning the model loop, tool execution coordination, and model-visible context inside `yo-core`. The effective binding MUST select one admitted Model Connector and exact API dialect before a Turn starts. A Model Connector MUST own only remote request and stream protocol and MUST translate its dialect into the connector-neutral round observations consumed by the loop. Neither `yo-cli`, a frontend, nor the connector may become the agent-loop owner. The same loop admits the separately contracted OpenAI Responses, provider-neutral OpenAI Chat Completions, and Kimi Chat Completions connectors through their dialect-derived identities. It MUST NOT probe another dialect, fall back to another connector, or branch on Provider identity.

For each accepted Turn, the backend MUST project the committed semantic Session history plus the new user input into the selected API dialect. Text deltas MUST become `ModelWork` Activities through the existing message segmentation and terminal-seal path. A model function call MUST preserve its wire call identity, function name, and exact accumulated argument bytes. It MUST become a correlated Tool Activity even when validation rejects it; invalid JSON, schema mismatch, unknown or duplicate identity, unavailable tool, and argument-bound failure MUST terminate that Activity with the typed validation failure and no effect. Validation MUST succeed before approval, admission, or dispatch. Approval and execution MUST use the frozen registry, admission policy, and execution-host boundary; the model service MUST NOT directly execute local workspace tools.

The backend MUST record each function call and its exact tool outcome before submitting a corresponding function-call output through the selected dialect in the next model request. Multiple calls returned by one response MAY execute concurrently only when the tool scheduler proves their approval and mutable resource leases independent; otherwise they MUST execute in model order. Results MUST be returned in stable call order regardless of execution completion order. A missing, duplicate, or mis-correlated call or result MUST fail the Turn.

The loop continues across model response, local tool execution, and tool-result submission until the model emits a final assistant message, cancellation is accepted, a bounded model-round limit is reached, or a typed failure occurs. One active Turn remains the Session limit. Cancellation MUST stop outstanding connector work promptly, prevent new tool execution, seal active Activities as interrupted, and run explicit connector and tool cleanup.

The loop owns any absolute model-request work deadline. That deadline MUST be optional and MUST default to absent. When the agent supplies it, the deadline MUST begin once for one logical model request, MUST cover every bounded connector-internal retry for that request, and MUST NOT reset on transport bytes, model output, decoded events, or retry. The next model request after a tool result, or a separately admitted request after an earlier failure, MUST receive a fresh deadline. A whole-Turn wall-clock budget is a separate optional cancellation policy and MUST NOT be inferred from the per-request deadline. Absence of either absolute budget MUST NOT disable the connector's finite transport-progress, event-delivery, data, round-count, cancellation, or cleanup bounds. Runtime deadline policy MUST remain outside the effective binding and MUST NOT open a binding epoch.

Provider response IDs, cache handles, and conversation IDs MAY be retained as diagnostic correlation but MUST NOT be the only continuation locator. A Yo-managed binding MUST explicitly declare `exact_replay` with `local_client` as the current executor rather than provider-native resume, and its complete effective profile MUST carry the exact `replay_profile`. Executable continuation MUST reconstruct the replay boundary named by the newest durable Continuation Anchor from the Session Journal and open a new binding epoch when endpoint, API dialect, Provider, Account, Model, connector identity, or replay profile changes. A committed mid-Turn function call, tool result, partial stream, private assistant fragment, or other suffix beyond that Anchor MUST remain diagnostic and MUST NOT become automatic continuation input. When no durable Anchor exists, the Session MUST follow the continuation contract's read-only fallback rather than constructing replay input. Exact replay MUST preserve message roles and order, exact visible text, function-call and tool-result relationships, and the recorded system/tool contract. `semantic-only/v1` forbids provider-private items. `kimi-private-local-plaintext/v1` declares schema `kimi.assistant-message/v1alpha1` and MAY preserve its item only under the Connector's lossless validation, visibility exclusion, byte bounds, binding scope, durable encoding, and exact request projection. Such an item is Session replay authority only for the same exact binding identity and replay profile; it never becomes generic visible history, provider-native state, or a frontend observation. Provider cache state and every uncontracted private field remain excluded.

No Continuation Anchor may cover a partial model stream, an uncommitted tool result, an uncertain request, a failed final response, or a K3 round whose required private assistant item is missing or not durably committed with its semantic replay delta. Usage and the exact effective binding MUST be attributed to the model response that produced them, including when the model changes inside one Yo Session.

The selected model catalog entry MUST provide an input-token limit, an output reserve, and the exact tokenizer profile used by an injected token counter. Every model request MUST pass that counter before dispatch, including every provider-private item that the selected Connector will send. Provider-side implicit caching MAY reduce billing but MUST NOT change exact replay or context admission. When exact replay or its private extension no longer fits either model-context or replay byte bounds, the backend MUST return typed `context_exhausted`, complete the current Turn as non-resumable, and reject a later Turn on that binding. It MUST NOT silently discard, truncate, redact, or summarize required private state. Lossy compaction remains an independently reviewed, user-visible handoff that opens a new binding epoch.

Tool arguments and outputs MUST pass the local tool boundary's semantic-admission gate before they become Activities, later model input, or a replay delta. A provider-private assistant item MUST come only from the selected Connector's successfully completed, correlated response; the backend validates its schema, epoch, bounds, and exact visible projection without interpreting or displaying its reasoning bytes. The backend MUST persist visible and private replay together as one semantic replay record and MUST NOT attach either payload to the payload-free resumable-outcome correlation record. Private bytes remain in the user-only local Session Repository, are not encrypted by the first implementation, and MUST be excluded from Transcript, Request trace, debug formatting, logs, errors, and diagnostics.

A future `managed_server` executor MAY load the same validated replay prefix and assemble the next model request on a Yo-managed Session service. It does not define a second replay meaning and MUST use the same replay contract, ordering, bounds, and Anchor boundary as `local_client`. It remains deferred until its remote repository, identity, digest, availability, and retention evidence has an independently reviewed implementation. The current backend MUST NOT advertise it.

## Rationale

Owning the loop in `yo-core` provides a genuinely native backend while preserving the existing frontend-independent Session contract. An explicit connector boundary lets multiple API dialects, including a Provider-specific dialect when its wire behavior materially differs, share the semantic loop without adding Provider branches or weakening their distinct grammars. Agent ownership of optional work budgets permits intentionally long model work without weakening transport-stall detection or binding identity. Exact semantic replay avoids coupling durable continuation to a provider's temporary response retention and keeps tool side effects correlated with Yo's own authority. Treating K3 reasoning as a separately typed, non-observable replay attachment preserves that boundary while adapting Yo to the current model instead of disabling it merely because its continuation grammar is richer.
