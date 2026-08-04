---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.appearance.activity-motion-profile
revision: sha256:499e5d926c3cc7d57e1c0724522192d35b6b61f9f3c1ab053990e7e854dc2509
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:d097e1eadba4bd41d5ffbf346564ee66fb6c188d760d2b61a20b9eb94c7f6bb0
---
# Korean Review Projection

## Translation

초기 내장 모션은 논리 프레임마다 120ms를 사용합니다. Rich marker는 `· ✢ ✳ ✶ ✻ ✽ ✽ ✻ ✶ ✳ ✢ ·`, ASCII marker는 `. *` 순서로 움직이며, 각 marker는 동일한 셀 폭을 가져야 합니다.

같은 프레임은 보이는 `Working` 문구나 선택 패널이 typed activity로 명시한 제목 상태에서 강조되는 grapheme 하나를 차례로 이동시킬 수 있습니다. 이 sheen은 style만 바꾸며 글자, 셀 폭, 행과 패널의 geometry, fitting 결과, 입력 동작, 중단 안내를 바꾸면 안 됩니다. activity가 아닌 제목 상태는 정적이어야 하고 marker와 sheen은 같은 elapsed sample을 사용합니다.

유효한 frame 하나만 가진 profile은 marker와 sheen 모션을 모두 끄며 timed redraw를 켜지 않습니다. 보이는 sheen이 두 grapheme보다 짧아 다음 phase에서 셀이 바뀔 수 없어도 redraw를 요청하지 않습니다. 한 committed appearance snapshot과 revision이 한 논리 frame의 marker cycle과 period를 함께 공급하고, frame 준비 중 replacement는 다음 완전한 frame부터 적용됩니다.
