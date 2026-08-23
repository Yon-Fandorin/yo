---
schema: methexis.knowledge/v1alpha1
id: agent.observability.view-projections
kind: decision
owner: agent-runtime
sources:
  - id: agent.observability-002
    revision: sha256:008d9a2d5989ccaa6a4c559b50cf539701a462ec9e104e0f8bc56320cc606248
relations:
  depends_on:
    - agent.observability.session-journal
  constrained_by:
    - agent.core.frontend-independent-boundary
    - tui.crate.ui-only-boundary
---
# Chat, Transcript, Request, and Usage projections

## Statement

Chat and Transcript MUST derive their displayed history from the read-only
semantic Session Journal. Request MUST be a read-only diagnostic projection
that joins the Journal's bounded correlation and availability records with
optional Request Audit detail under the same Session lifecycle. Neither the
projection nor its detail may become an independent authority. Chat MUST
remain the editable default interaction surface. Its exposure policy MUST
follow established coding-agent
interaction: show concise intent, meaningful tool and file activity, tests,
approvals, failures, and results while collapsing repetitive exploration and
long output.

Transcript MUST be the transparent chronological superset of Chat and add
detailed semantic and Activity lifecycle, context, failures, and explicit
observation or persistence gaps. Request MUST be a full-page read-only
diagnostic trace that presents all of the Journal's bounded correlation and
availability records for the full Session in chronological order, not a
request-list browser. It
MUST show the observable backend exchange, revisions, attempts, outcomes,
redaction, exact observation boundary, and a typed reason when detail is
unavailable. An interactive surface MAY highlight the context currently viewed
in Chat or Transcript within that trace; a highlighted context with no direct
request MUST say so instead of selecting a nearby request. Returning across
linked views MUST restore each view's cursor and scroll state. A future remote
reader MAY fetch detail on demand only after a real remote consumer defines
that contract; this decision does not create a remote Request Audit interface.

Session Usage MUST be a read-only projection of usage receipts from completed
ModelWork Activities only. Archived CLI Usage and live F4 Usage MUST consume
one shared typed projection and carry identical meaning; neither frontend may
independently decode or aggregate receipts. The projection MUST preserve
receipt chronology. Each token aggregate MUST be complete, partial, or
unavailable. Partial and unavailable aggregates MUST expose covered/total
receipt coverage (x/y) so missing values do not appear complete. Cache-read
share MUST include only receipts that explicitly report cache-read token data
and have a known input-token denominator. Its token denominator MUST contain
known input tokens from only those eligible receipts, and it MUST expose
eligible/total receipt coverage. A Session with no recognized completed
receipts MUST succeed with an empty projection. For recognized receipt schemas,
reported zero, absent, and unsupported MUST remain distinct, while malformed
data MUST fail the whole projection closed. Codex aggregation MUST use
per-turn usage only and MUST exclude cumulative thread_total. Usage MUST NOT
infer cost, billing, cache hits, uncached tokens, missing attribution, or
cross-provider cache-write equivalence, and MUST NOT expose raw request,
response, credential, or private-reasoning content.

## Rationale

One semantic replay source keeps concise work and transparent chronology
aligned, while optional correlated detail permits wire-level diagnosis across
TUI and future GUI frontends without forcing it into the semantic Journal. A
shared typed Usage projection likewise keeps archived and live presentations
semantically aligned without duplicating receipt interpretation.
