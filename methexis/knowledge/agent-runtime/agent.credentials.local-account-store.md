---
schema: methexis.knowledge/v1alpha1
id: agent.credentials.local-account-store
kind: decision
owner: agent-runtime
sources:
  - id: agent.credentials-001
    revision: sha256:17ffd481e980ffd3cd0ed5fc79822a4c2a4e5383ed8341b00d192915461bd122
relations:
  depends_on:
    - agent.model.service-binding
  constrained_by:
    - agent.observability.session-journal
---
# Local account credential store

## Statement

API credentials MUST be stored separately from ordinary Yo settings. The first
implementation MUST read a dedicated `credentials.yaml` beside the selected Yo
`config.yaml`. Its versioned shape maps stable `AccountId` values to secret
material and MUST NOT duplicate Provider or Model routing policy. The file MUST
be opened once with no-follow semantics. The exact opened handle MUST identify
a regular file and every other object type MUST be rejected. Current-user
ownership and the absence of group or world permission bits MUST be checked on
that same handle before reading. Reads MUST remain size bounded, MUST use only that
handle, and MUST reject a file whose identity or relevant metadata changes
during capture. A path pre-check or a second path-based open MUST NOT satisfy
these requirements.

Environment variables MUST NOT provide API keys. The process MUST read and
validate the credential file once during startup and keep an immutable
in-memory `CredentialStore` for that process. Runtime reload, refresh, account
rotation, failover, interactive login, and OS keychain integration are
deferred. Absence of a selected Account or its credential MUST fail before a
model request and MUST NOT fall through to another Account.

Ordinary configuration and UI may expose Account ID and display name. The
connector MUST receive only an opaque resolved secret for the exact selected
Account. Secret types MUST redact `Debug` and display output. API keys MUST NOT
enter diagnostics, logs, Session Journal records, Request Audit data, model
binding evidence, command-line arguments, or child-process environments.

The credential resolver MUST be injected into startup assembly rather than
opened by a Model Connector. A later tenant-aware caller MAY choose which
AccountId to resolve before this boundary, but the first implementation MUST
not persist or display tenant state.

## Rationale

A separate permission-restricted file avoids putting long-lived secrets in
shell environments or shareable settings. Startup-only resolution keeps file
access and secret ownership outside protocol code while preserving a narrow
future seam for another credential store.
