---
schema: methexis.knowledge/v1alpha1
id: methexis.validation.check-classes
kind: rule
owner: methexis
sources:
  - id: methexis.validation-model.check-classes
    revision: sha256:2b337bbed69e8ec1b51cae1c8361208cca53e7a45828dcb086ab54b81dc1acc3
relations:
  depends_on:
    - methexis.approval.current-record
    - methexis.checkpoint.selection
    - methexis.validation.snapshot-construction
  validated_by:
    - tools/methexis/tests/check.rs::local_failures_are_aggregated_and_block_global_validation
    - tools/methexis/tests/cli.rs::check_only_accepts_comma_lists_and_repeated_flags_equivalently
    - tools/methexis/tests/cli.rs::check_only_rejects_unknown_and_empty_selectors
  applies_to:
    - tools/methexis/src/check/runner.rs::check_repository_selected
    - tools/methexis/src/cli.rs::parse_check_selection
---
# Ordered validation check classes

## Statement

Fast Check MUST expose four ordered classes:

```text
records -> relations -> authority -> artifacts
```

The default request MUST select every class. An explicit selection MUST execute
every prerequisite of each requested class. The report MUST distinguish
canonical `requested_checks` from `executed_checks` and mark every planned
class `passed`, `failed`, or `blocked`. A failed prerequisite MUST block
all remaining dependent classes rather than presenting them as executed.

Explicit selection MUST accept repeatable comma-separated class names.
Names are case-sensitive; surrounding whitespace is ignored. Unknown names and
empty comma segments MUST be usage failures. A blocked requested class MUST
make the overall validation unsuccessful because the requested work did not
complete.
