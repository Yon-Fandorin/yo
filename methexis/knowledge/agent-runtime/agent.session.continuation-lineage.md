---
schema: methexis.knowledge/v1alpha1
id: agent.session.continuation-lineage
kind: decision
owner: agent-runtime
sources:
  - id: agent.session-001
    revision: sha256:304b4e64d8bb64499cf6bf09ff2b4e08025852ffeaa6dd57a785744888353092
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

Every backend binding MUST declare exactly one versioned continuation strategy.
`exact_replay` MUST declare an executor of `local_client` or `managed_server`;
`backend_managed_state` MUST NOT declare a replay executor. Strategy is an
explicit binding capability and MUST NOT be inferred from backend kind, Provider,
API dialect, or model name. It is distinct from the binding-transition mode:
for example, a newly opened binding can be seeded by an `exact_replay` transition
and then continue using either declared strategy. An exact-replay binding MUST
carry the complete effective binding's `replay_profile` in its binding evidence.
`semantic-only/v1` forbids private replay; `kimi-private-local-plaintext/v1`
declares `kimi.assistant-message/v1alpha1`. The profile is part of binding
identity and epoch freshness and MUST NOT be inferred from ModelId. The
format-compatibility contract's exact legacy omission decodes only as
`semantic-only/v1`.

Both exact-replay executors share one semantic replay contract, validation, and
Anchor boundary. The executor changes only where the validated prefix is loaded
and the next model request is assembled. `local_client` reconstructs that prefix
from the local Session Repository. `managed_server` reserves the same operation
for a future Yo-managed Session service; it MUST NOT be advertised until the
remote repository identity, replay boundary, content and contract digests,
binding epoch, availability, and retention are verified by an independently
reviewed implementation.

A backend using `backend_managed_state` MUST reconnect through the anchor's
versioned locator and verify the returned backend identity. Successful backend-managed resume continues the same Yo Session and binding.
Yo still owns the durable transcript, semantic events, correlation evidence, and
locator, while the backend owns the model-visible conversational state. Such an
Anchor MUST NOT claim or reference a Yo replay delta. If continuation under the
recorded strategy fails but a
replacement backend supports exact semantic replay, yo MAY create a new backend
binding inside the same Yo Session and seed it from the committed semantic
boundary. Exact semantic replay preserves message roles and order, exact
committed text, tool-call and tool-result relationships, and every other
backend-visible semantic record required by the target adapter; it does not
claim to restore provider caches or identical future output. A binding whose
replay profile declares provider-private replay MUST additionally preserve every
required private item losslessly through the same Anchor and atomic replay
commit. That item is eligible only when the resumed binding has the same exact
binding identity and replay profile; it MUST NOT be projected as generic
history. If the source Anchor covers any private item and the target binding or
profile cannot consume that exact item, the transition is never `exact_replay`:
it requires the separately approved `lossy_handoff` path unless an independently
reviewed lossless conversion contract exists. The same rule covers K3 effort or a K2.7 Code ModelId or speed-tier change,
as well as any endpoint, connector, replay-profile, or schema change, even when
the target itself does not require private state. Missing, wrong-epoch, unbounded, or wrong-schema
source state likewise makes exact replay unavailable rather than lossy by
omission. The binding transition, backend and model identities, replay boundary,
private-replay availability, and known cache loss MUST be recorded.

If only a lossy handoff is possible, yo MUST open the saved Session read-only,
describe the missing or transformed context, and ask once before continuing.
Explicit approval MAY create a replacement binding inside the same Yo Session,
but the Journal MUST record a visible context-loss boundary and the original
durable history MUST remain intact. Yo MUST NOT silently perform a lossy
handoff, resend an uncertain request, or describe a replacement binding as
native resume. Provider-private replay contents MUST remain hidden during that
disclosure; the operator sees only its schema, presence, byte count, and whether
the target can preserve it.

Persisting `summarize-older-semantic-history/v1` as the Session's versioned context strategy selects the standing automatic policy for the backend's separately bounded compaction handoff in that Session. Yo MUST show the resulting lossy boundary and measurements but MUST NOT interrupt every pressure event for another confirmation. This is the sole exception to the preceding per-handoff approval rule and covers only the exact source Anchor and boundary, visible summary, retained semantic suffix, dropped-private disclosure, and successor epoch durably committed by that compaction contract. `exact-replay-only/v1` permits no automatic loss. Provider, Model, connector, endpoint, replay-profile, schema, or any other replacement-driven lossy handoff MUST still open read-only, describe the loss, and obtain one explicit confirmation; it MUST NOT reuse the automatic-compaction policy.

## Rationale

The user task should not acquire a new identity merely because its execution
provider changed. Stable Yo Session identity plus explicit binding epochs keeps
that continuity honest, while a durable anchor and visible context-loss
boundary prevent retries, partial writes, cache loss, and backend replacement
from being mistaken for stronger continuation than yo can provide. Explicit
epoch ownership makes the cost of task-level identity visible to every Journal
consumer rather than hiding it behind a Session ID.
