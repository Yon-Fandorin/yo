---
schema: methexis.knowledge/v1alpha1
id: methexis.knowledge.body-contract
kind: rule
owner: methexis
sources:
  - id: methexis.knowledge-model.body-contract
    revision: sha256:23ce9a091096b3363f6f45285c06e29768339e546e44221504a6c1cd42721232
relations:
  depends_on:
    - methexis.knowledge.kind-vocabulary
    - methexis.knowledge.record-format
  validated_by:
    - tools/methexis/src/check.rs::tests::headings_inside_fenced_code_do_not_satisfy_body_sections
    - tools/methexis/src/check.rs::tests::headings_inside_html_comments_make_the_body_invalid
    - tools/methexis/src/check.rs::tests::raw_html_spelling_inside_fenced_code_is_allowed
  applies_to:
    - tools/methexis/src/check.rs::validate_metadata
---
# Canonical body contract

## Statement

Every canonical KU body MUST have a non-empty `Statement` section. A decision
MUST additionally have a non-empty `Rationale` section. A procedure MUST
additionally have non-empty `Steps` and `Completion Criteria` sections.

Canonical bodies MUST NOT contain raw HTML blocks or HTML comments. Content
inside a code fence or hidden rendered content MUST NOT satisfy a required
semantic section.
