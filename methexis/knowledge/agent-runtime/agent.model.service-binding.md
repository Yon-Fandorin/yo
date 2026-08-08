---
schema: methexis.knowledge/v1alpha1
id: agent.model.service-binding
kind: decision
owner: agent-runtime
sources:
  - id: agent.model-001
    revision: sha256:d2fc8098664ec115a875f2e5bbd9df261d05f397544212c8823fcf76eb587b55
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.session.continuation-lineage
---
# Model service binding identity

## Statement

Model routing MUST use distinct typed `ProviderId`, `AccountId`, and `ModelId` identities. Each identity MUST be stable within the local configuration while its optional display name MAY change without changing identity. A Provider is the group that offers models through one model-service policy; it is not an Agent Backend. An Account selects one locally configured credential and entitlement scope within a Provider. The exact `ProviderId` and `AccountId` pair selects that credential. A Model selects the exact model name sent on the wire.

The external configuration keys that select these coordinates MUST be named `provider`, `account`, and `model`; internal code MUST carry their typed IDs. A complete effective model binding MUST identify its Provider, Account, Model, Model Connector, explicit API dialect, and normalized endpoint. A binding change in any of those fields MUST open a new backend binding epoch before another Turn starts. Durable attribution MUST use stable identities, never display names, and MUST exclude credentials.

`OpenAI-compatible` names an API family only. Configuration and runtime code MUST use the exact `api_dialect`; the initial supported value is `openai-responses`. An API dialect defines the complete request, response, streaming-event, tool-call, usage, and terminal grammar. HTTPS, SSE framing, authentication, deadlines, and decoding are connector implementation responsibilities rather than the dialect identity itself. Chat Completions and Responses MUST NOT be collapsed into one dialect or selected by probing and fallback.

OpenRouter and QwenCloud are the initial first-class configured Providers. They MUST share the provider-neutral Yo-managed backend and matching dialect connector. Provider-specific endpoint, credential, Model, and tokenizer-profile values belong to catalog entries and conformance evidence; neither Provider may become a backend kind or connector identity.

Tenant selection, tenant UI, account rotation, and account failover are deferred. The first implementation MUST NOT add a `TenantId` or tenant field. Binding and credential resolution MUST nevertheless remain injected boundaries so a later caller-owned tenant scope can select a Provider-and-Account pair without redefining Provider, Account, Model, dialect, or backend identity.

## Rationale

Separating service group, credential scope, wire Model, dialect grammar, and connector implementation prevents a display label or vendor name from becoming routing authority. Exact binding identity also makes Provider and Model changes auditable while allowing multiple OpenAI-compatible Providers to share one backend architecture.
