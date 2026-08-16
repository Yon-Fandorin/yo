---
schema: methexis.knowledge/v1alpha1
id: agent.credentials.local-account-store
kind: decision
owner: agent-runtime
sources:
  - id: agent.credentials-001
    revision: sha256:38f14d10813091792438cc0754a6b245f3523a816ad7adad9b9691ade5edcdf1
relations:
  depends_on:
    - agent.model.service-binding
  constrained_by:
    - agent.observability.session-journal
---
# Provider-scoped local credential store

## Statement

API credentials are separate from public settings. The first local store is a pre-version `credentials.yaml` beside the selected Yo `config.yaml`, namespaced first by stable ProviderId and then AccountId. It uses the shared bounded safe-Rust YAML parser and carries no top-level format-version field; a retired `version` field is unknown and fails before secret use or mutation. Yo provides no legacy decoder, automatic migration, dual write, downgrade shim, or automatic deletion for preceding local bytes; the diagnostic requires the operator to back up or remove stale local state and register the connection again. Coordinates select only secret material; the same AccountId under different Providers remains independent and duplicate exact pairs fail.

Capture occurs only when the effective binding declares an external credential requirement. It opens once with no-follow semantics, validates the immutable exact pair, and fails before a request when absent. A binding without that requirement, including Local Codex, needs no credential path. There is no fallback. The opened handle must be a regular file owned by the current user with no group or world permission bits; reads are bounded, use only that handle, and reject identity or relevant-metadata change.

An absent path is a canonical empty snapshot with reserved opaque revision token `absent` and no pairs. Under the store lock, `prepare` re-reads the snapshot and binds one exact pair action to its expected CredentialRevision and one freshly reserved non-absent planned CredentialRevision; preparation changes no repository bytes. The planned revision is independently generated, never derived from a secret or file bytes, and may be durably recorded by the connection orchestrator before commit. A prepared mutation is one exact expected-revision, planned-revision, pair, and add-or-replace-or-remove intent and cannot be retargeted.

`commit` accepts only that prepared mutation and, for add or replace, the still in-memory secret. It rejects a state other than the expected revision or the exact planned revision. Observing the planned revision with the intended pair action already applied is idempotent success; any other winner is conflict. First creation rechecks absence, creates an exclusive same-directory current-user-owned regular temporary with mode `0600`, durably writes complete current-shape bytes carrying the planned revision, and atomically publishes only while the expected revision remains `absent`. Existing mutation performs the same bounded complete replacement and atomic publication against its exact expected revision. An unexamined winner is never overwritten, and failure removes only the operation's temporary.

Success leaves one complete old or new snapshot. Exactly one pair changes, unrelated pairs remain byte-equivalent, and an exact already-applied replacement or removal is idempotent. Removing the final pair may publish a canonical empty current-shape file with its planned non-absent revision, but must not return to reserved `absent`, which means the path has never been created or is currently missing.

CredentialRevision is a private opaque CAS and recovery receipt. It may occur only in the permission-restricted local credential snapshot, the secret-safe store API, and the permission-restricted redacted connection-operation journal owned by the model service contract. It is excluded from user-visible partial outcomes, general diagnostics and logs, Session Journal, Request Audit, binding evidence, and public configuration. The store API exposes internally only prepare or commit status and the exact expected and planned opaque revisions; it never returns secret bytes. Binding verification, operation locking, public-repository ordering, command-local config composition, and cross-repository recovery belong to the model service contract.

Environment variables, command-line secret values, standard input, and child processes never supply keys. Interactive setup reads through a controlling-TTY no-echo channel. Non-interactive external connect accepts one `--credential-file PATH` only together with explicit `--yes` authorization. The path is a locator rather than secret material. Yo opens the final path exactly once with no-follow semantics and requires that handle to be a current-user-owned regular file with mode exactly `0400` or `0600`. It rejects an initial size over 16,386 bytes, reads the complete file through EOF with one additional byte of overflow detection, and accepts only when the captured length equals the stable before-and-after size at most 16,386 bytes. It rejects identity, size, ownership, permission, or relevant timestamp change during the read, removes at most one terminal LF or CRLF, and then requires 1 to 16,384 bytes of valid UTF-8 accepted by `ApiCredential`. It never trims any other byte, follows a final symlink, reads a replacement pathname, or sends file contents to diagnostics. The source file is never modified or deleted on success or failure. Secret types redact display and debug output, and a Connector receives only the opaque secret for its exact pair. Runtime reload, refresh, failover, keychain integration, and other non-interactive secret channels are deferred. The injected resolver preserves a future tenant-owned selection seam without adding TenantId, tenant fields, or tenant UI now.

## Rationale

Reserving the independently generated CredentialRevision before mutation lets a write-ahead recovery record distinguish the exact planned winner from unrelated change without deriving identity from secret bytes. The reserved absent revision still closes first-install CAS without confusing a deliberately empty existing store with a missing one. A narrow owner-only file channel works with agent-created temporary files and mounted secret files without placing the key in process arguments, environment, inherited input, or terminal scrollback; preserving every non-line-ending byte avoids silently changing credential identity.
