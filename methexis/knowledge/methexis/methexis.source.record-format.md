---
schema: methexis.knowledge/v1alpha1
id: methexis.source.record-format
kind: rule
owner: methexis
sources:
  - id: methexis.source-model.record-format
    revision: sha256:b31839e9e0d0d613a385cde233bc7ea8aaf14d1ba854f838f2cda3608d699cda
relations:
  depends_on:
    - methexis.source.kind-vocabulary
  validated_by:
    - tools/methexis/src/source/tests.rs::closed_payload_schema_parses_all_kinds_and_rejects_unknown_fields
    - tools/methexis/src/source/tests.rs::source_loader_rejects_duplicate_ids_before_context_freshness_mapping
  applies_to:
    - tools/methexis/src/model.rs::SourceRecord
    - tools/methexis/src/source/records.rs
    - tools/methexis/src/source/validation.rs
---
# Source record format

## Statement

Each Source MUST be stored as one typed YAML record below
`methexis/sources/<kind>/`. The schema is closed and MUST NOT accept catch-all
payloads or unknown fields. The stable semantic `SourceId` read from record
content is identity; directory and filename are mutable organizational hints
and MUST NOT define identity.

The record MUST own its original content or external locator exactly once in a
kind-specific payload. A code Source MUST contain a safe repository-relative
path, a non-empty symbol, and a lowercase SHA-256 content hash. Its optional
line hint is only a navigation aid: path and symbol locate the Source, the
content hash detects drift, and the symbol is not a byte-range extraction
boundary.
