---
schema: methexis.knowledge/v1alpha1
id: methexis.context.build-reuse
kind: rule
owner: methexis
sources:
  - id: methexis.context-model.build-reuse
    revision: sha256:9acf2b2fd99e56811836261598244d39f352dee753a8e3d949bb87987efc4e30
relations:
  depends_on:
    - methexis.context.build-publication
    - methexis.context.payload-manifest
    - methexis.context.source-freshness
---
# ContextBuild reuse, export, and opt-in deep verification

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

A separate opt-in deep verification operation, exposed by the Pilot as
`verify-context-build <request.json> <sha256:BuildId>`, MAY accept one exact
Context Resolution request and one expected BuildId. It MUST capture the
request and resolve current trusted authority. It MUST compile that captured
request in an independent, non-publishing path that does not consult the named
managed build or reuse any of its files or bytes until final comparison, then
require the derived BuildId plus the managed `context.md` and `manifest.json`
closed file set to match exactly. Immediately before success it MUST revalidate
the request, every observed mutable Source, the trusted ref, the active
Checkpoint, and the named managed build's directory identity, closed file set,
path types and symlink state, and both artifact bytes through the same
whole-operation consistency boundary used by resolution. Success returns a
bounded structured result identifying the BuildId, current trusted commit, and
artifact hashes.

The verifier reproduces only from the supplied request and current trusted
authority; it is not a historical-authority reconstruction API. Invalid BuildId
syntax, a derived identity mismatch, a missing or non-regular managed build,
extra or missing files, symlinked paths, changed artifact bytes, or a concurrent
input, authority, or managed-build change MUST fail without an eligible result.
Verification MUST NOT create, replace, quarantine, or otherwise mutate a
ContextBuild. A repository-local serialization lock MAY be used without making
verification a publication operation.
