---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.crate.ui-only-boundary
revision: sha256:890ea5c07e508f9b29a01edeb5e274fffa4750e3cfe22d1fc1ef53546988d2a4
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:34a0c7c4ed3cd51bd04c08f6c1ebfa288e3aff0bce78059ea9cc742db33afcff
---
# Korean Review Projection

## Translation

초기 `yo-tui` 프로덕션 크레이트는 UI 동작만 소유해야 하며, 좁은 퍼사드를 노출하고 구현 세부 사항은 기본적으로 크레이트 내부에서만 보이게 해야 합니다.

이 경계는 애플리케이션 및 제품 의미를 터미널 표현과 분리하면서도, 근거 없는 크레이트 분할을 피합니다.
