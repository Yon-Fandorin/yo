---
schema: methexis.knowledge/v1alpha1
id: agent.observability.session-journal
kind: decision
owner: agent-runtime
sources:
  - id: agent.observability-001
    revision: sha256:f219e1c793d56890f0f7c96a7927339d25c4d6d0f9ece745346e99bd3eb87f57
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.runtime.session-turn-activity
    - agent.storage.session-repository
---
# Durable session observation journal

## Statement

One ordered durable Session Journal MUST be the semantic replay source for
Session history and views. A process-local Live Projection MUST own only the
uncommitted tail required for responsive streaming. Backend transport deltas
MUST update that Live Projection immediately without becoming replay authority by
themselves. The Session worker MUST be the sole writer owner. TUI, GUI, and
other frontends MUST consume read-only views that merge the durable prefix with
the live tail by stable item identity.

Exact text MUST be accumulated without interpretation, inserted whitespace, or
other content changes into immutable ordered segments. Agent-message segments
MUST be forced when buffered UTF-8 text reaches 16 KiB, when its oldest
uncommitted byte reaches one second, at a non-text ordering boundary, or when
the message terminates. Tool-output segments MUST use 64 KiB and the same
one-second age, non-text boundary, and termination rules. A size split MUST
preserve valid UTF-8, and joining the segments MUST reproduce the exact original
text. Segment boundaries
are persistence detail and MUST NOT change Chat or Transcript message meaning.

Every message termination observed by the runtime MUST be sealed with a typed
`MessageEnded` outcome of `completed`, `interrupted`, or `failed`. It MUST carry
the segment count and total UTF-8 byte count required to detect an incomplete
reconstruction. When durable append is available, the final non-empty tail and
`MessageEnded` MUST be committed atomically without duplicating the complete
body. When durable append is unavailable, the same final tail and
`MessageEnded` MUST be published atomically as explicitly volatile Live
Projection state. That volatile terminal seal MUST NOT become replay authority
or a Continuation Anchor. A later complete Session snapshot MUST include the
sealed message and becomes authoritative only after durable publication.
During recovery, a durably recorded message with no terminal record MUST be
sealed as interrupted before later durable events are accepted and MUST remain
visibly partial rather than becoming a completed message.

The Journal MUST also record backend-neutral semantic Session, Turn, and
Activity events together with bounded, payload-free Request correlation and
availability records. Stable operation identity,
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

Separating transient presentation from durable semantic replay preserves
responsive streaming without turning arbitrary backend chunks into the Session
contract. Bounded immutable segments limit crash loss and record size, while a
terminal seal distinguishes complete output from a recoverable partial
message. Stable meaning and bounded correlation remain backend-independent as
optional Request Audit detail evolves.
