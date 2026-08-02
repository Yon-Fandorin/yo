---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.appearance.activity-motion-profile
revision: sha256:836bae25a568d52e178b9f5e2711296bc23aafd07a279642b0bf40ad9f4082b0
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:423d781433d11ce1012e92d13f801f5b178be4428c9db7a7ea567ad0fed73b83
---
# Korean Review Projection

## Translation

초기 내장 activity marker는 논리 frame 하나를 정확히 120ms 유지합니다. Rich는 `· ✢ ✳ ✶ ✻ ✽ ✽ ✻ ✶ ✳ ✢ ·`, ASCII는 `. *` 순서로 반복합니다.

각 profile의 모든 frame은 비어 있지 않고 제어 문자가 없는 렌더 가능한 확장 grapheme 하나여야 하며, 같은 profile 안에서는 cell 폭이 모두 같아야 합니다. 빈 frame 목록, 0ms period, 잘못된 frame, 서로 다른 폭은 publication 전에 거부합니다.

애니메이션은 장식 activity marker만 변경하고 별도로 공급되는 marker 이외의 문구, 폭 맞춤 동작, 중단 안내를 그대로 보존합니다. 유효한 frame 하나만 가진 profile도 표현할 수 있고 timed redraw를 켜지 않으므로, runner를 바꾸지 않고 추후 reduced-motion 선택을 열 수 있습니다.

하나의 committed appearance snapshot과 revision이 논리 frame에서 선택과 paint에 쓰는 marker cycle·period를 함께 제공합니다. frame 준비 중 replacement가 일어나도 다음 완전한 frame부터 적용합니다.

이 profile을 활성화하면 이미 승인되었지만 비활성인 `tui.appearance.frame-consistency`와 그 의존성 `tui.appearance.session-publication`도 함께 선택됩니다. 이 더 넓은 eligibility 전환은 별도 activation 검수에서 명시적으로 확인해야 합니다.
