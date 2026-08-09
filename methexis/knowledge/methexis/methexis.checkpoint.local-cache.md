---
schema: methexis.knowledge/v1alpha1
id: methexis.checkpoint.local-cache
kind: rule
owner: methexis
sources:
  - id: methexis.checkpoint-model.local-cache
    revision: sha256:bd30358de195bcc0c9458df7051a5ea3ab0a1358c75be61a5f1c8082baa5eaa4
relations:
  depends_on:
    - methexis.checkpoint.activation-transition
---
# Local active-Checkpoint cache

## Statement

Any local active pointer MUST be only a reconstructible, non-authoritative
cache. It MUST bind the exact Git tree identity and trusted active-Checkpoint
hash from which it was derived, be replaced crash-safely, and be discarded
rather than used when either identity mismatches.

A local cache MUST NOT grant approval, activation, or eligibility and MUST NOT
replace the tracked active record. Concurrent authority changes are serialized
by repository merge and review; a runtime database lock MUST NOT be presented
as the authority boundary.
