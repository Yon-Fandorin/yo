---
schema: methexis.knowledge/v1alpha1
id: methexis.source.kind-vocabulary
kind: definition
owner: methexis
sources:
  - id: methexis.source-model.kind-vocabulary
    revision: sha256:7855ca492cc3d92200038aa913860fc0a86dd64d725819b991a3b4495bb3040a
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

The canonical English body is agent-generated and begins as Draft. When Korean
user input is material provenance, a reviewer sees an authorized Source excerpt
and a generated Korean review projection. Full transcripts MUST NOT be retained
by default. Tracked conversation Sources contain only a minimal relevant
excerpt, redact sensitive content, and require explicit human authorization.
Sensitive provenance MAY remain outside Git behind an opaque reference and
content hash. English efficiency is a measured Pilot hypothesis, not a
permanent product assumption.
