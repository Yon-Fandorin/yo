---
schema: methexis.knowledge/v1alpha1
id: methexis.context.build-reuse
kind: rule
owner: methexis
sources:
  - id: methexis.context-model.build-reuse
    revision: sha256:94b6207953e240d1795b80c7dbe2f1b106209ae95d7791ae9451a9bba1755655
relations:
  depends_on:
    - methexis.context.build-publication
    - methexis.context.payload-manifest
    - methexis.context.source-freshness
---
# ContextBuild reuse and export

## Statement

The fixed BuildId store owns the immutable original in the Pilot. A successful
structured result returns `created` or `reused`, the BuildId, and the paths and
hashes of both artifacts. That per-operation result also records the exact
current trusted commit observed for final verification; it may therefore differ
across safe reuse of the same immutable build. Cache reuse first reproduces the
BuildId plan, verifies current freshness, and verifies the stored manifest and
artifact hashes. Existing different content at the same BuildId is corruption
and MUST NOT be overwritten.

Caller-selected output paths are not part of initial resolution. A later
read/export operation MAY stream a verified artifact to stdout or copy it to a
caller-selected destination without changing the managed original, BuildId,
lineage, or integrity checks.
