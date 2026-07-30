---
schema: methexis.knowledge/v1alpha1
id: agent.observability.session-journal
kind: decision
owner: agent-runtime
sources:
  - id: agent.observability-001
    revision: sha256:f62ef22e05e57dfd570a4c70707d5b6685baf84ebf6f7f3bcfe442d02c892e9e
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.runtime.session-turn-activity
    - agent.storage.session-repository
---
# Durable session observation journal

## Statement

One ordered durable Session Journal MUST be the replay source for session
views and diagnostic history. It MUST record backend-neutral semantic Session,
Turn, and Activity events together with correlated backend-specific exchange
events. A backend exchange envelope MUST identify backend kind and version,
observation boundary, direction, payload schema, and operation correlation.
Requests, responses, notifications, server-initiated requests, retries, and
terminal outcomes MUST remain distinguishable.

Semantic meaning MUST be committed at capture time and MUST NOT later be
reconstructed solely by interpreting an old backend wire payload. A
backend-specific payload MAY be retained for Request diagnosis, but an unknown
or unsupported payload MUST remain an explicit unavailable observation rather
than blocking replay of unrelated semantic records. Redaction MUST happen
before durable admission. Credentials, complete environment variables, private
reasoning values, and other prohibited raw values MUST NOT enter the Journal;
removal MUST be represented explicitly when it affects interpretation.

## Rationale

Capturing stable meaning and backend-specific evidence together permits
backend-independent history while retaining the exact boundary needed to debug
Codex app-server today and deeper direct-model transports later.
