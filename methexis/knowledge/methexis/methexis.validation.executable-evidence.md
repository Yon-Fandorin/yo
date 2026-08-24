---
schema: methexis.knowledge/v1alpha1
id: methexis.validation.executable-evidence
kind: rule
owner: methexis
sources:
  - id: methexis.validation-model.executable-evidence
    revision: sha256:a72b4ed17d68437438ae3c4170fd3d87fdde5c613c98ef74ef5fdca1739a0479
---
# Executable evidence activation guard

## Statement

Checkpoint activation additionally verifies:

- approval and Source freshness;
- complete required dependency closure;
- exclusion of replaced old knowledge;
- current executable evidence; and
- reproducible evidence for each approval's declared review basis.

Canonical-basis approval evidence reproduces the exact canonical English `RevisionId` directly and requires no Projection. Projection-basis evidence additionally reproduces the exact referenced human-review Projection. Missing, malformed, or mismatched evidence for the selected basis fails activation closed; an unreferenced optional Projection does not participate.

Executable evidence is content addressed. Unchanged code, knowledge, command, and tool inputs reuse prior evidence. Related changes stale only affected evidence. Context resolution consumes an active Checkpoint and does not rerun the entire validation suite, but it MUST run the freshness guard defined by `SOT-007` before using cached eligibility.
