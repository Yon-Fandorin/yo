---
schema: methexis.knowledge/v1alpha1
id: agent.session.continuation-lineage
kind: decision
owner: agent-runtime
sources:
  - id: agent.session-001
    revision: sha256:57db178254bbd4eb9334fd039adf4f02bad7994e62e27a0fc6a99648b1b9e3dd
relations:
  depends_on:
    - agent.backend.execution-topology
    - agent.observability.session-journal
    - agent.runtime.session-turn-activity
    - agent.storage.session-repository
---
# Session continuation and lineage

## Statement

History viewing and executable continuation MUST remain separate capabilities.
A Continuation Anchor MUST identify an accepted backend request, its correlated
stable resumable outcome, the fully committed semantic Journal boundary, and
the versioned backend Session locator needed to continue. Resume MUST select
the newest durable Continuation Anchor. Those identities and the locator are
bounded Session Journal correlation data, not optional Request Audit detail.
Request payloads, headers, revision or attempt evidence, and other diagnostic
detail MUST NOT be required to construct or validate an Anchor. Incomplete,
unaccepted, or uncommitted suffixes MUST remain diagnostic evidence and MUST
NOT become automatic continuation input.

When no durable Continuation Anchor exists, yo MUST open the saved Session
read-only. It MAY offer an explicitly confirmed empty child Session that
records its parent and the absence of a source anchor, but it MUST NOT replay
or resend the uncommitted suffix. A recovery snapshot MAY support a later
Continuation Anchor only after durable publication completes and every anchor
condition is satisfied; the snapshot alone MUST NOT create one.

A backend advertising Native Resume MUST reconnect through the anchor's
versioned Session locator and verify the returned identity. If native resume
fails, yo
MUST open the saved Session read-only and ask whether to continue in a new
Session. Only an explicit user choice MAY create that child Session. The new
Session MUST record its parent and source anchor, keep the parent immutable,
and seed only the committed semantic boundary through a backend-supported
exact replay. Any unsupported replay fidelity MUST be disclosed before the
new Session starts. Yo MUST NOT silently create a replacement Session, resend
an uncertain request, or present lineage continuation as native resume.

## Rationale

The latest stable Request remains the understandable user-facing resume point,
while an explicit anchor prevents retries, partial writes, and backend loss
from being mistaken for a safe continuation state.
