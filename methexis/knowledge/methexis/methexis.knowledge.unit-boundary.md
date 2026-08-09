---
schema: methexis.knowledge/v1alpha1
id: methexis.knowledge.unit-boundary
kind: rule
owner: methexis
sources:
  - id: methexis.knowledge-model.unit-boundary
    revision: sha256:cf9fd831a871505d557e1508b2e9c1b45f24c5535c82f095cf34aebb38354617
relations:
  depends_on:
    - methexis.knowledge.unit
---
# Knowledge unit boundary

## Statement

Conditions, outcomes, and exceptions that would be incomplete alone MUST
remain in the same KU. Reusable definitions and behavior that can change
independently MUST use separate KUs.
