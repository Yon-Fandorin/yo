---
schema: methexis.knowledge/v1alpha1
id: methexis.source.revision-identity
kind: rule
owner: methexis
sources:
  - id: methexis.source-model.revision-identity
    revision: sha256:3aab2c4332313d28befa2df6cb931dc852a4b1b87cce7b8d0efe3a41b2970ab6
relations:
  depends_on:
    - methexis.source.record-format
  validated_by:
    - tools/methexis/src/source/tests.rs::source_revision_excludes_code_line_hint
    - tools/methexis/src/source/tests.rs::source_revision_is_domain_separated_by_kind
  applies_to:
    - tools/methexis/src/source/revision.rs::calculate
---
# Source revision identity

## Statement

`SourceRevision` MUST be encoded as `sha256:<lowercase-hex>` over one
domain-separated, length-delimited representation containing the Source
schema, SourceId, kind, and every semantic field for that kind.

YAML formatting, physical record path, generation time, a code line hint, and
the record's revision field MUST NOT affect SourceRevision. Code path, symbol,
and content hash are semantic and MUST affect it. Decision and authorized
excerpt content, opaque reference and hash, and each external freshness mode's
locator or reference, hash, version, and expiry fields MUST affect it when
present in that payload.
