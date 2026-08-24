---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.validation.working-tree-authority
revision: sha256:4862bc9daa856e66e1bf2c198b00f5b422a1282deaa695bafc662fa969f98ec3
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:6f1d2dc598ce9d97bdb704555f2e985b050fb22f3d77c4634a0ef92d5514a035
---
# Korean Review Projection

## Translation

# Working tree validation은 권한이 아니다

## 명세

`methexis.validation.snapshot-construction`이 소유한 구조 record validation이 성공한 뒤, working-tree Fast Check는 각 approval proposal을 현재 Draft Knowledge와 typed Source에 대해 명시된 review basis에 따라 평가해야 한다. Canonical 근거 승인은 정확한 canonical 영문 `RevisionId`에만 일치하며 Projection을 요구하지 않는다. Projection 근거 승인은 정확한 현재 Projection profile, compiler, hash도 요구한다. 참조되지 않은 stale Projection은 부적격 증거이며 일치하는 canonical proposal을 막아서는 안 된다. malformed record와 approval이 참조하는 Projection evidence는 계속 fail closed한다. Local evidence는 matching, stale, missing으로 보고할 수 있지만 trusted approval 또는 activation을 부여해서는 안 된다. 이 unit은 구조 record validation을 재정의해서는 안 된다.

Fast Check는 `methexis.status.approval`이 도출한 approval axis와 `methexis.status.eligibility`가 도출한 최종 eligibility를 소비해야 하며 둘 중 어느 것도 재정의해서는 안 된다. 현재 working tree 또는 host 관찰은 해당 status contract가 연결한 demotion guard를 통해서만 기여할 수 있고 Draft, inactive, unapproved content를 승격해서는 안 된다.

성공한 report는 trusted integration에서 파생된 status를 포함하더라도 자신의 authority를 Draft로 식별해야 한다. Trusted status 평가가 실패하면 Fast Check도 실패해야 하며 local proposal evidence를 trusted state의 fallback으로 사용해서는 안 된다.

저장소 로컬 `refs/heads/develop`에서 도달 가능한 record만 현재 Pilot의 approval authority이다. Task input, environment variable, 호출 agent가 이를 override해서는 안 된다. Operation 시작 시 ref는 하나의 정확한 commit으로 한 번 resolve되며, 그 pinned snapshot만 계산에 사용하는 authority이고 모든 결과에 해당 commit을 기록한다. 최종 authority 안정성을 약속하는 operation은 반환 전에 configured ref와 active-record identity만 다시 읽을 수 있다. 불일치는 pinned operation을 실패시키며 더 새로운 snapshot으로 전환하지 않는다. 격리된 test는 내부 주입 policy를 사용할 수 있지만 production input surface는 아니다.

Authority read는 caller Git configuration과 environment를 제거한 system Git executable을 사용해야 한다. Replacement ref와 graft 유사 object substitution을 비활성화하여 기록된 object ID와 materialized tree가 달라질 수 없게 해야 한다.

Caller가 제공한 Task commit, proposed Slice commit, working-tree state, branch name은 결코 authority가 아니다. 사람이 승인한 Wave commit을 임시 trust anchor로 지원하는 일은 저장소 policy가 caller가 제어할 수 없는 configuration surface를 소유할 때까지 미룬다.

Knowledge, Source, approval, Checkpoint, active-Checkpoint record는 tracked 상태여야 한다. Projection은 선택적 Projection review 분기를 위해 명시적으로 생성된 경우에만 tracked 상태가 된다. Proposed branch와 working-tree edit는 저장소 approval workflow가 configured trust anchor에 통합할 때까지 Draft input이다. Database나 local file은 재구축 가능한 index 또는 cache일 수 있지만 두 번째 writable authority가 되어서는 안 된다. Compiler는 storage-neutral immutable `KnowledgeSnapshot`을 소비한다.
