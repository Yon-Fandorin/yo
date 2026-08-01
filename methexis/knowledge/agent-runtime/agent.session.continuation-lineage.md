---
schema: methexis.knowledge/v1alpha1
id: agent.session.continuation-lineage
kind: decision
owner: agent-runtime
sources:
  - id: agent.session-001
    revision: sha256:773ebf9bfa8615c41114a33abc1d57d93a1729f2ad3d8fc7e9ef95f8b12dc6a8
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.observability.session-journal
    - agent.runtime.session-turn-activity
    - agent.storage.session-repository
---
# Session continuation and lineage

## Statement

A Yo Session is the stable, durable identity of one user task. It MUST use a
UUIDv7 and MUST remain the same when its Agent Backend, backend Session locator,
transport, or model changes. Those replaceable execution details belong to
ordered, versioned backend binding epochs recorded inside the Yo Session. Every
binding transition MUST close the previous epoch before opening the next, and
every Continuation Anchor MUST identify its epoch. Journal consumers MUST
preserve epoch boundaries and MUST NOT replay, summarize, or attribute backend
state as though one binding spanned a transition.

Only an intentional user fork creates a new Yo Session. A fork MUST record its
parent and either its source anchor or the explicit absence of one. An empty
child offered when no durable anchor exists is such a fork, not a backend
reconnection. A non-empty fork MUST seed its first binding from the source
anchor through a verified backend-native fork or through the same exact-replay
and explicitly approved lossy-handoff rules used for replacement bindings.

History viewing and executable continuation MUST remain separate capabilities.
A Continuation Anchor MUST identify an accepted backend request, its correlated
stable resumable outcome, the fully committed semantic Journal boundary, and
the versioned backend binding and locator needed to continue. Those identities
and the locator are bounded Session Journal correlation data, not optional
Request Audit detail. Resume MUST select the newest durable Continuation Anchor
and MUST NOT fall back to an older binding locator: an older backend lacks the
later committed history and may only become a replacement binding through the
replay rules below. Incomplete, unaccepted, or uncommitted suffixes MUST remain
diagnostic evidence and MUST NOT become automatic continuation input.
Request payloads, headers, revision or attempt evidence, and other Request Audit
detail MUST NOT be required to construct or validate an Anchor.

When no durable Continuation Anchor exists, yo MUST open the saved Session
read-only. It MAY offer an explicitly confirmed empty child Session that
records its parent and the absence of a source anchor, but it MUST NOT replay
or resend the uncommitted suffix. A recovery snapshot MAY support a later
Continuation Anchor only after durable publication completes and every anchor
condition is satisfied; the snapshot alone MUST NOT create one.

A backend advertising Native Resume MUST reconnect through the anchor's
versioned locator and verify the returned backend identity. Successful native
resume continues the same Yo Session and binding. If native resume fails but a
replacement backend supports exact semantic replay, yo MAY create a new backend
binding inside the same Yo Session and seed it from the committed semantic
boundary. Exact semantic replay preserves message roles and order, exact
committed text, tool-call and tool-result relationships, and every other
backend-visible semantic record required by the target adapter; it does not
claim to restore provider caches, hidden provider state, or identical future
output. The binding transition, backend and model identities, replay boundary,
and known cache loss MUST be recorded.

If only a lossy handoff is possible, yo MUST open the saved Session read-only,
describe the missing or transformed context, and ask once before continuing.
Explicit approval MAY create a replacement binding inside the same Yo Session,
but the Journal MUST record a visible context-loss boundary and the original
durable history MUST remain intact. Yo MUST NOT silently perform a lossy
handoff, resend an uncertain request, or describe a replacement binding as
native resume.

## Rationale

The user task should not acquire a new identity merely because its execution
provider changed. Stable Yo Session identity plus explicit binding epochs keeps
that continuity honest, while a durable anchor and visible context-loss
boundary prevent retries, partial writes, cache loss, and backend replacement
from being mistaken for stronger continuation than yo can provide. Explicit
epoch ownership makes the cost of task-level identity visible to every Journal
consumer rather than hiding it behind a Session ID.
