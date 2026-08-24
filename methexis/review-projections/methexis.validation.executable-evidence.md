---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.validation.executable-evidence
revision: sha256:a8d987733520168b2c8b4cf9b03ac6d2fe3e062b9972f4fdf98d7afcc56f90ad
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:05719ffb79cb9608faba1674b1255e6c1e3e1aa5f19dd1d6b9798186b4df8826
---
# Korean Review Projection

## Translation

# Executable evidence activation guard

## 명세

Checkpoint activation은 다음 사항도 검증한다.

- approval과 Source freshness
- 완전한 required dependency closure
- 교체된 이전 Knowledge의 제외
- 현재 executable evidence
- 각 approval이 선언한 review basis에 대해 재현 가능한 evidence

Canonical 근거 approval evidence는 정확한 canonical 영문 `RevisionId`를 직접 재현하며 Projection을 요구하지 않는다. Projection 근거 evidence는 참조된 정확한 human-review Projection도 재현한다. 선택된 basis의 evidence가 missing, malformed, mismatched이면 activation은 fail closed하며, 참조되지 않은 optional Projection은 참여하지 않는다.

Executable evidence는 content-addressed이다. 바뀌지 않은 code, knowledge, command, tool input은 이전 evidence를 재사용한다. 관련 변경은 영향받은 evidence만 stale로 만든다. Context resolution은 active Checkpoint를 소비하며 전체 validation suite를 다시 실행하지 않지만, cached eligibility를 사용하기 전에 `SOT-007`이 정의한 freshness guard를 실행해야 한다.
