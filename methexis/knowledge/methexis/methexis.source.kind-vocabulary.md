---
schema: methexis.knowledge/v1alpha1
id: methexis.source.kind-vocabulary
kind: definition
owner: methexis
sources:
  - id: methexis.source-model.kind-vocabulary
    revision: sha256:b34736641a50ef1d2e851b828e732e79d5afe353bcde18f519eefbb24b23dbd8
relations:
  depends_on:
    - methexis.knowledge.unit
  validated_by:
    - tools/methexis/src/source/tests.rs::closed_payload_schema_parses_all_kinds_and_rejects_unknown_fields
    - tools/methexis/src/source/tests.rs::conversation_and_external_sources_fail_closed_in_a_multi_source_unit
  applies_to:
    - tools/methexis/src/model.rs::SourcePayload
---
# Source kind vocabulary

## Statement

The initial Source vocabulary is closed to these four kinds:

- `decision` records an accepted design decision;
- `code` records a repository path, symbol, and exact content hash;
- `conversation` records an authorized minimal excerpt or an opaque reference;
- `external` records a document or standard outside the repository.

Conversation material MUST be either an authorized excerpt or an opaque
reference with a content hash. External freshness MUST be declared as
immutable, mutable, or attested. Conversation and External Sources MUST remain
ineligible until a verifier for the corresponding kind and freshness mode
exists.
