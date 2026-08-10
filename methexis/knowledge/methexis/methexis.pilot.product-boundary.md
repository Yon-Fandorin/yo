---
schema: methexis.knowledge/v1alpha1
id: methexis.pilot.product-boundary
kind: rule
owner: methexis
sources:
  - id: methexis.pilot-model.product-boundary
    revision: sha256:9b189457ffde60e1782173deb99979dd8d7a40c4d35847d32a3a661c996c9edf
---
# Methexis Pilot and product boundary

## Statement

`tools/methexis` MUST begin as an internal `yo` Pilot. Its first job is to
improve code-agent work on `yo`; it is not yet a generic knowledge platform.

`yo` is the incubation testbed and first reference consumer, not the expected
permanent owner of Methexis. Repository extraction and domain generalization
are separate gates: validated Pilot capabilities MAY move to a standalone
Methexis repository, while generalizing beyond the `yo`-proven contract
requires evidence from a second real product consumer.

A small SOT operating-procedure corpus MUST provide a structurally different
secondary sample. It MAY reference the repository workflow authority but MUST
NOT restate or become a second canonical owner for `CONTRIBUTING.md` policy.
Existing workflow rules remain references or generated projections; new
KnowledgeUnits own only SOT-specific procedures not already owned elsewhere.

The domain model MUST NOT contain TUI-specific fields. General domain expansion
beyond the `yo`-proven contract requires a second real non-`yo` product
consumer.
