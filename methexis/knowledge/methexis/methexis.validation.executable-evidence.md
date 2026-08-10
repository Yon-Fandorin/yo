---
schema: methexis.knowledge/v1alpha1
id: methexis.validation.executable-evidence
kind: rule
owner: methexis
sources:
  - id: methexis.validation-model.executable-evidence
    revision: sha256:87ad0c637b5c3e22a40ea1c5865995e28bc2eb260f283edc1d2876e1dcebb195
---
# Executable evidence activation guard

## Statement

Checkpoint activation additionally verifies:

- approval and Source freshness;
- complete required dependency closure;
- exclusion of replaced old knowledge;
- current executable evidence;
- reproducible human-review projection.

Executable evidence is content addressed. Unchanged code, knowledge, command,
and tool inputs reuse prior evidence. Related changes stale only affected
evidence. Context resolution consumes an active Checkpoint and does not rerun
the entire validation suite, but it MUST run the freshness guard defined by
`SOT-007` before using cached eligibility.
