---
schema: methexis.knowledge/v1alpha1
id: agent.credentials.local-account-store
kind: decision
owner: agent-runtime
sources:
  - id: agent.credentials-001
    revision: sha256:df2bf50c8cee5491dd7b895385de7463d8a566ccec756b7fe6523a86d776496f
relations:
  depends_on:
    - agent.model.service-binding
  constrained_by:
    - agent.observability.session-journal
---
# Provider-scoped local credential store

## Statement

API credentials MUST be stored separately from ordinary Yo settings. The first implementation MUST read a dedicated `credentials.yaml` beside the selected Yo `config.yaml`. Its versioned shape MUST namespace secret material first by stable `ProviderId` and then by stable `AccountId`. Those coordinates select a credential only and MUST NOT duplicate endpoint, Model, connector, API dialect, or display-name routing policy. The same `AccountId` MAY exist under different Providers and MUST resolve independently; a duplicate exact Provider-and-Account pair MUST be rejected.

The file MUST be opened once with no-follow semantics. The exact opened handle MUST identify a regular file and every other object type MUST be rejected. Current-user ownership and the absence of group or world permission bits MUST be checked on that same handle before reading. Reads MUST remain size bounded, MUST use only that handle, and MUST reject a file whose identity or relevant metadata changes during capture. A path pre-check or a second path-based open MUST NOT satisfy these requirements.

Environment variables MUST NOT provide API keys. The process MUST read and validate the credential file once during startup and keep an immutable in-memory `CredentialStore` keyed by the exact Provider-and-Account pair for that process. Startup assembly MUST resolve the pair from the selected effective model binding. Absence of that exact pair or its credential MUST fail before a model request and MUST NOT fall through to another Account or Provider. Runtime reload, refresh, account rotation, failover, interactive login, and OS keychain integration are deferred.

Ordinary configuration and UI may expose Provider and Account IDs and display names. The connector MUST receive only an opaque resolved secret for the exact selected pair. Secret types MUST redact `Debug` and display output. API keys MUST NOT enter diagnostics, logs, Session Journal records, Request Audit data, model binding evidence, command-line arguments, or child-process environments.

The credential resolver MUST be injected into startup assembly rather than opened by a Model Connector. A later tenant-aware caller MAY choose the Provider-and-Account pair inside its own tenant scope before this boundary, but the first implementation MUST NOT add a `TenantId`, tenant field, or tenant UI.

## Rationale

Provider-scoped account coordinates prevent a common local Account ID such as `default` from selecting another Provider's secret. A separate permission-restricted file avoids long-lived shell secrets, while startup-only injected resolution preserves a narrow future seam for tenant-owned or alternative credential stores.
