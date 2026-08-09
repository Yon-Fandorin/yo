---
schema: methexis.knowledge/v1alpha1
id: methexis.status.negative-record
kind: rule
owner: methexis
sources:
  - id: methexis.status-model.negative-record
    revision: sha256:1f3ef80419ea75defef7247402a6fd6a04a116f93674aa4a7f78633a2689f297
relations:
  depends_on:
    - methexis.knowledge.revision-identity
---
# Durable negative status records

## Statement

Durable review holds and invalidations MUST be tracked records bound to the
exact affected Knowledge revision. A review hold MUST supply the `suspect`
guard condition for unresolved semantic or provenance uncertainty. An explicit
human invalidation MUST supply the `invalid` guard condition.

Neither record MUST grant approval or activation. Each record MUST supply
machine-readable evidence for its condition so that precedence and transitions
remain testable.
