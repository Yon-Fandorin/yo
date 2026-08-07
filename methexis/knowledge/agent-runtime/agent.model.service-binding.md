---
schema: methexis.knowledge/v1alpha1
id: agent.model.service-binding
kind: decision
owner: agent-runtime
sources:
  - id: agent.model-001
    revision: sha256:a1c115321da2a857e78f51f6aed83bdc86a3a28d9a87578e2554e084b66a3ece
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.session.continuation-lineage
---
# Model service binding identity

## Statement

Model routing MUST use distinct typed `ProviderId`, `AccountId`, and `ModelId`
identities. Each identity MUST be stable within the local configuration while
its optional display name MAY change without changing identity. A Provider is
the group that offers models through one model-service policy; it is not an
Agent Backend. An Account selects one locally configured credential and
entitlement scope within a Provider. A Model selects the exact model name sent
on the wire.

The external configuration key that selects an account MUST be named
`account`; internal code MUST carry `AccountId`. A complete effective model
binding MUST identify its Provider, Account, Model, Model Connector, explicit
API protocol, and normalized endpoint. A binding change in any of those fields
MUST open a new backend binding epoch before another Turn starts. Durable
attribution MUST use stable identities, never display names, and MUST exclude
credentials.

`OpenAI-compatible` names a protocol family only. Configuration and runtime
code MUST use the exact `api_protocol`; the initial supported value is
`openai-responses`. Chat Completions and Responses MUST NOT be collapsed into
one protocol value or selected by probing and fallback.

Tenant selection, tenant UI, account rotation, and account failover are
deferred. The first implementation MUST NOT add a `TenantId` or tenant field.
Binding and credential resolution MUST nevertheless remain injected boundaries
so a later caller-owned tenant scope can select an Account without redefining
Provider, Account, Model, or backend identity.

## Rationale

Separating service, credential scope, model, and protocol prevents a display
label or vendor name from becoming routing authority. Exact binding identity
also makes model changes and continuation auditable without implementing
tenant behavior prematurely.
