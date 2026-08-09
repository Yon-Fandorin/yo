---
schema: methexis.knowledge/v1alpha1
id: methexis.validation.bounded-success-output
kind: rule
owner: methexis
sources:
  - id: methexis.validation-model.bounded-success-output
    revision: sha256:b01c11543a1e0cef64774b1c4076261dcd15b6db0b851d071000d399d7a5551c
relations:
  depends_on:
    - methexis.validation.check-classes
  validated_by:
    - tools/methexis/tests/cli/bounded_check.rs::check_summary_returns_a_bounded_success_report
    - tools/methexis/tests/cli/bounded_check.rs::check_unit_keeps_only_the_requested_unit_in_a_summary
    - tools/methexis/tests/cli/bounded_check.rs::check_unit_rejects_unknown_and_duplicate_ids
    - tools/methexis/tests/cli/bounded_check.rs::check_unit_rejects_unbounded_and_pre_authority_combinations
    - tools/methexis/tests/cli/bounded_check.rs::check_summary_preserves_the_full_failure_report
  applies_to:
    - tools/methexis/src/cli.rs::run_check
    - tools/methexis/src/cli.rs::parse_check_selection
---
# Bounded successful validation output

## Statement

An agent MAY request bounded successful output with `--summary`. The summary
MUST retain requested and executed classes, their outcomes, authority, affected
IDs, and diagnostic count while omitting the complete Knowledge list.

Exactly one `--unit <knowledge-id>` occurrence MAY be used only when summary
output is enabled and the request includes the `authority` or `artifacts`
class. After validation successfully reaches that authority-capable stage, a
known unit selection MUST return only that exact unit, while an unknown ID MUST
be a usage failure rather than an empty success. More than one `--unit`
occurrence and incompatible selector combinations MUST be usage failures before
validation.

Bounding MUST NOT hide failure evidence. An underlying validation failure MUST
take precedence over unit resolution and return the complete ordinary report
and diagnostics regardless of summary or unit selectors.
