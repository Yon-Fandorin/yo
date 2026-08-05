---
schema: methexis.knowledge/v1alpha1
id: agent.observability.view-projections
kind: decision
owner: agent-runtime
sources:
  - id: agent.observability-002
    revision: sha256:5b84c15abb860fb9b2f402d2b90fce6b3c613adfdb28fdcc40e5c4fb62ae6ec9
relations:
  depends_on:
    - agent.observability.session-journal
  constrained_by:
    - agent.core.frontend-independent-boundary
    - tui.crate.ui-only-boundary
---
# Chat, Transcript, and Request projections

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

## Rationale

One semantic replay source keeps concise work and transparent chronology
aligned, while optional correlated detail permits wire-level diagnosis across
TUI and future GUI frontends without forcing it into the semantic Journal.
