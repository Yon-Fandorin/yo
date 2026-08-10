---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.validation.working-tree-authority
revision: sha256:62e0a7be69c7f88535e5f40002a5d4574fcca48c11f3229a5e91b95bb97d3e98
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:2a3fb33520afe53076becade0077d6a253375cb094d37956d835cf1c46c0cfbb
---
# Korean Review Projection

## Translation

`methexis.validation.snapshot-construction`이 소유한 구조 record 검증이 성공한 뒤, working-tree Fast Check는 현재 Draft Knowledge와 typed Source를 기준으로 한국어 review Projection과 approval proposal을 평가해야 합니다. proposal evidence를 `matching_proposal`, `stale_proposal`, missing으로 보고할 수 있지만 로컬 evidence가 trusted approval이나 activation을 부여하면 안 됩니다. 이 unit은 구조 record 검증을 다시 정의하면 안 됩니다.

Fast Check는 `methexis.status.approval`이 파생한 approval axis와 `methexis.status.eligibility`가 파생한 최종 eligibility를 소비해야 하며 어느 상태도 다시 정의하면 안 됩니다. 현재 working tree 또는 host observation은 해당 status 계약이 연결하는 demotion guard를 통해서만 기여할 수 있고 Draft, inactive, unapproved content를 승격하면 안 됩니다.

성공 보고서는 trusted integration에서 파생된 상태를 포함하더라도 보고서 자체의 authority를 Draft로 식별해야 합니다. trusted status 평가가 실패하면 Fast Check도 실패를 반환해야 하며 로컬 proposal evidence를 trusted state로 대체하면 안 됩니다.

현재 Pilot의 approval authority는 repository-local refs/heads/develop에서 도달 가능한 record뿐입니다. operation 시작 시 ref를 exact commit으로 한 번 pin하고 그 snapshot만 사용하며 결과에 commit을 기록합니다. final stability recheck mismatch는 새 snapshot으로 전환하지 않고 실패합니다. Authority read는 caller Git 설정과 환경을 제거한 system Git을 사용하고 replacement ref와 graft substitution을 막습니다. Caller가 준 Task/Slice commit, working tree, branch는 authority가 아닙니다. Knowledge, Source, approval, Checkpoint와 active record는 tracked여야 하며 database/local file은 rebuildable index/cache일 뿐 writable authority가 될 수 없습니다. Compiler는 storage-neutral immutable KnowledgeSnapshot을 소비합니다.

### 전체 개정 정본 원문 대조

# Working-tree validation is not authority

## Statement

After structural record validation owned by
`methexis.validation.snapshot-construction` succeeds, working-tree Fast Check
MUST evaluate Korean review Projections and approval proposals against the
current Draft Knowledge and typed Sources. It MAY report proposal evidence as
`matching_proposal`, `stale_proposal`, or missing, but local evidence MUST NOT
grant trusted approval or activation. This unit MUST NOT redefine structural
record validation.

Fast Check MUST consume the approval axis derived by
`methexis.status.approval` and final eligibility derived by
`methexis.status.eligibility`; it MUST NOT redefine either status. Current
working-tree or host observations MAY contribute only through the demotion
guard routed by those status contracts and MUST NOT promote Draft, inactive, or
unapproved content.

A successful report MUST identify its own authority as Draft even when it
includes statuses derived from trusted integration. If trusted status
evaluation fails, Fast Check MUST return failure and MUST NOT fall back to
local proposal evidence as trusted state.

Records reachable from the repository-local `refs/heads/develop` are the only
approval authority in the current Pilot. Task input, environment variables, and
the invoking agent MUST NOT override it. At the start of an operation, the ref
is resolved once to an exact commit; that pinned snapshot is the only authority
used for computation and its commit is recorded in every result. An operation
that promises final authority stability MAY reread only the configured ref and
active-record identities before returning. A mismatch fails the pinned
operation and never switches it to the newer snapshot. An internal injected
policy MAY be used by isolated tests but is not a production input surface.

Authority reads MUST use the system Git executable with caller Git
configuration and environment removed. Replacement refs and graft-like object
substitution MUST be disabled, so the recorded object ID and materialized tree
cannot diverge.

A Task commit, proposed Slice commit, working-tree state, or branch name
supplied by the caller is never authority. Supporting a human-approved Wave
commit as a temporary trust anchor is deferred until repository policy owns a
non-caller-controlled configuration surface.

Knowledge, Source, approval, Checkpoint, and active-Checkpoint records MUST be
tracked. Proposed branch and working-tree edits are Draft inputs until the
repository approval workflow integrates them into the configured trust anchor.
A database or local file MAY be a rebuildable index or cache, but MUST NOT
become a second writable authority. The compiler consumes a storage-neutral
immutable `KnowledgeSnapshot`.
