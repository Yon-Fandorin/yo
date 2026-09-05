---
schema: methexis.knowledge/v1alpha1
id: methexis.status.eligibility
kind: definition
owner: methexis
sources:
  - id: methexis.status-model.eligibility
    revision: sha256:bbd7100ffb2bded9f6012d57b27ab353c854e1d3e17238a8a4220910ea21bfdf
relations:
  depends_on:
    - methexis.checkpoint.activation-transition
    - methexis.status.approval
    - methexis.status.demotion-evidence
  validated_by:
    - tools/methexis/tests/checkpoint_flow/contract.rs::trusted_activation_becomes_active_when_decision_sources_are_fresh
    - tools/methexis/tests/checkpoint_flow/contract.rs::trusted_code_activation_degrades_without_losing_approval_on_byte_drift
  applies_to:
    - tools/methexis/src/check/runner.rs::check_repository_selected
    - tools/methexis/src/source.rs::Eligibility
---
# Derived eligibility status

## Statement

Final eligibility MUST be derived rather than authored after the
pre-transition status guard and trusted active-record transition. Its closed
states and winning conditions are:

- `invalid`, `suspect`, or `stale`: the corresponding winning condition from
  `methexis.status.demotion-evidence`;
- `inactive`: no ineligible guard condition wins and the trusted active
  Checkpoint does not select the revision; and
- `active`: no ineligible guard condition wins and the trusted active
  Checkpoint selects the revision.

Final precedence MUST preserve the guard order and then membership order:
`invalid > suspect > stale > inactive > active`.

Every final eligibility state MUST include machine-readable evidence for its
winning condition. `invalid`, `suspect`, and `stale` MUST preserve the winning
guard evidence. `inactive` and `active` MUST identify the exact trusted active
Checkpoint and whether that Checkpoint omitted or selected the revision.

Normal context MUST require both `approved` approval and `active`
eligibility. Every other combination MUST be excluded. Suspect and stale
content MUST remain visible in a marked diagnostic view and MUST NOT be emitted
as normal context. Invalid content MUST NOT be emitted as context.
