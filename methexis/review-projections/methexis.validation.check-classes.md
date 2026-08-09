---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.validation.check-classes
revision: sha256:4e5791dfccd2e501ff52ffa7dbd188a8f2a9b1a38aa6c845a8b68e2816118bee
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:020b89fe2280a4a1d02db8c0c2046d67a60093e377c81463c368dee2efa1b4f9
---
# Korean Review Projection

## Translation

Fast Check는 `records -> relations -> authority -> artifacts` 순서의 네 class를 제공해야 합니다.

기본 요청은 모든 class를 선택해야 합니다. 명시적 선택은 요청한 각 class의 모든 prerequisite를 실행해야 합니다. 보고서는 canonical `requested_checks`와 `executed_checks`를 구분하고 계획된 각 class를 `passed`, `failed`, `blocked` 중 하나로 표시해야 합니다. prerequisite가 실패하면 남은 dependent class는 실행했다고 표시하지 말고 차단해야 합니다.

명시적 선택은 반복 가능한 comma-separated class 이름을 받아야 합니다. 이름은 대소문자를 구분하고 주변 공백은 무시합니다. 알 수 없는 이름과 빈 comma segment는 usage failure여야 합니다. 요청된 class가 blocked라면 요청한 검증이 완료되지 않은 것이므로 전체 검증도 성공하면 안 됩니다.
