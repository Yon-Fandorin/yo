---
schema: methexis.knowledge/v1alpha1
id: methexis.validation.snapshot-construction
kind: rule
owner: methexis
sources:
  - id: methexis.validation-model.snapshot-construction
    revision: sha256:8326897b96a461bac9a504429d96b16c162c0235db1dc0347f19fbd92cf67b38
relations:
  depends_on:
    - methexis.knowledge.identity
    - methexis.knowledge.record-format
    - methexis.relation.required-graph
    - methexis.source.record-format
  validated_by:
    - tools/methexis/tests/check.rs::local_failures_are_aggregated_and_block_global_validation
    - tools/methexis/tests/check.rs::global_failures_include_missing_targets_and_cycles
    - tools/methexis/tests/check.rs::duplicate_knowledge_ids_are_reported_for_each_path
    - tools/methexis/tests/check.rs::repeated_checks_and_physical_relocation_preserve_identity
    - tools/methexis/tests/check.rs::authority_root_symlinks_are_rejected_without_following_them
    - tools/methexis/src/check.rs::tests::diagnostic_order_uses_location_before_message
  applies_to:
    - tools/methexis/src/check.rs::load_records
    - tools/methexis/src/check.rs::validate_global
    - tools/methexis/src/check/runner.rs::check_repository_selected
---
# All-or-nothing structural validation snapshot

## Statement

Fast editing validation MUST construct one all-or-nothing structural snapshot
in two ordered phases.

The local `records` phase MUST parse every discovered working-tree owner,
Source, Knowledge, and related record, including untracked additions under the
authority roots, and aggregate schema, field, identity, relation-shape, and
required-body diagnostics. The global `relations` phase MUST run only after
every record passes locally, then aggregate duplicate identities, missing
owners or relation targets, and graph cycles.

Discovery MUST reject authority roots, authority directories, and tracked
record paths that are symbolic links without following them. Snapshot revision
and unit identity MUST remain deterministic across repeated validation, record
discovery order, and physical relocation; canonical Knowledge identity remains
owned by `methexis.knowledge.identity`.

Diagnostics MUST use stable codes and deterministic phase, path, code, line,
column, message, and affected-ID ordering. Any local or global structural
diagnostic MUST prevent the report from carrying a snapshot revision or unit
set. A failed `records` phase MUST prevent the `relations` phase from
executing. Planning and blocking of later validation classes remain owned by
`methexis.validation.check-classes`.
