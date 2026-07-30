---
schema: methexis.knowledge/v1alpha1
id: agent.observability.session-journal
kind: decision
owner: agent-runtime
sources:
  - id: agent.observability-001
    revision: sha256:4fce4175ba7677de0194790b53efa61af6bbc15dfe3786f1a88ce6487316fb89
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.runtime.session-turn-activity
    - agent.storage.session-repository
---
# Durable session observation journal

## Statement

One ordered durable Session Journal MUST be the semantic replay source for
Session history and views. It MUST record backend-neutral semantic Session,
Turn, and Activity events together with bounded, payload-free Request
correlation and availability records. Stable operation identity,
accepted-request identity, correlated resumable outcome, backend kind and
version, observation boundary, exchange kind and direction, payload schema
identity, and the versioned backend Session locator required for continuation
belong to those Journal records; they are not Request detail. Requests, responses,
notifications, server-initiated requests, retries, and terminal outcomes MUST
remain distinguishable through correlation records even when detail is
unavailable.

Semantic meaning MUST be committed at capture time and MUST NOT later be
reconstructed solely by interpreting an old backend wire payload. A
backend-specific Request Audit detail, including request payloads, headers, and
revision or attempt evidence, is a logically distinct optional diagnostic
domain under the same Session Repository lifecycle. It MUST NOT become
semantic authority, and its absence MUST NOT block Journal replay or
Continuation Anchor validation. Missing, unsupported, volatile, or unpersisted
detail MUST remain explicit rather than blocking unrelated semantic records.
Redaction MUST happen before any durable detail admission. Credentials,
complete environment variables, private reasoning values, and other prohibited
raw values MUST NOT enter durable storage; removal MUST be represented
explicitly when it affects interpretation. Until that admission boundary is
implemented, Request detail MUST remain process-local and volatile.

## Rationale

Keeping stable meaning and bounded correlation in the Journal preserves
backend-independent replay and continuation while optional Request Audit
detail can evolve for Codex app-server and later direct-model transports
without becoming a second Session authority.
