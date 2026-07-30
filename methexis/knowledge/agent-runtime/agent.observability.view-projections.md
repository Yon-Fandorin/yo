---
schema: methexis.knowledge/v1alpha1
id: agent.observability.view-projections
kind: decision
owner: agent-runtime
sources:
  - id: agent.observability-002
    revision: sha256:375d5b05cb6592c0b29a38feba51f199e9b46d4b2e838f008b78e87d458a8f77
relations:
  depends_on:
    - agent.observability.session-journal
  constrained_by:
    - agent.core.frontend-independent-boundary
    - tui.crate.ui-only-boundary
---
# Chat, Transcript, and Request projections

## Statement

Chat, Transcript, and Request MUST derive their displayed history from the
same read-only Session Journal projection and MUST NOT become independent
authorities. Chat MUST remain the editable default interaction surface. Its
exposure policy MUST follow established coding-agent
interaction: show concise intent, meaningful tool and file activity, tests,
approvals, failures, and results while collapsing repetitive exploration and
long output.

Transcript MUST be the transparent chronological superset of Chat and add
detailed semantic and Activity lifecycle, context, failures, and explicit
observation or persistence gaps. Request MUST be a full-page read-only
projection anchored to the context currently viewed in Chat or Transcript,
not primarily a request-list browser. It MUST show the correlated backend
exchange, revisions, attempts, outcomes, redaction, and exact observation
boundary. A context with no direct request MUST say so instead of selecting a
nearby request. Returning across linked views MUST restore each view's cursor
and scroll state.

## Rationale

One replay source keeps concise work, transparent chronology, and wire-level
diagnosis aligned across TUI and future GUI frontends without forcing all
detail into the default conversation.
