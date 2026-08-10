---
schema: methexis.knowledge/v1alpha1
id: methexis.validation.tracked-artifacts
kind: rule
owner: methexis
sources:
  - id: methexis.validation-model.tracked-artifacts
    revision: sha256:f13b4e00381a390a533d7dc2fbeb53bf562984b9df93f17ca3e9697a73d43bcb
---
# Tracked artifact validation boundary

## Statement

The `artifacts` class validates only tracked contract artifacts derived from
trusted authority. In this Pilot it checks the registered context manifests'
Checkpoint ID, hash, and authority-basis commit against the active trusted
Checkpoint. It does not claim byte-for-byte regeneration and does not inspect
or gate rebuildable `.local-exclude/` ContextBuild caches. Generic Rust tests,
linting, and formatting remain Cargo and `hk` responsibilities rather than
Methexis check classes. A repository or isolated fixture with none of the
registered tracked artifact paths has an empty, passing `artifacts` class.
Presence of any registered path enables the closed set, after which every
registered artifact is required. If no active trusted Checkpoint is available,
`authority` may pass as an evaluation while `artifacts` is `blocked`; the
requested validation is incomplete, so the overall report fails and directs
the caller to establish active trusted authority.
