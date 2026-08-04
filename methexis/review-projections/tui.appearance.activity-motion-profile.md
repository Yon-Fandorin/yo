---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.appearance.activity-motion-profile
revision: sha256:5597da77815e00d30e2c2503f9cc2a1c94e87b0e99e1c58eafef9130f0eb3db2
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:dea6bad171bba3fa2282dd6e92d81f274081253275d2a443cf036c41a5b781a1
---
# Korean Review Projection

## Translation

초기 내장 모션은 논리 프레임마다 정확히 120ms를 사용합니다. Rich marker는 `· ✢ ✳ ✶ ✻ ✽ ✽ ✻ ✶ ✳ ✢ ·`, ASCII marker는 `. *` 순서로 움직이며, 한 profile의 모든 marker는 하나의 유효한 grapheme이면서 같은 셀 폭을 가져야 합니다.

내장 marker는 appearance가 결정하는 고정된 accent style을 사용하며 bold와 dim을 사용하지 않습니다. frame이 바뀌어도 글자 굵기는 바뀌지 않습니다. 이는 별 모양에 추가적인 굵기 왜곡을 만들지 않기 위한 것이며, 터미널 폰트 자체의 glyph 돌출까지 제어한다고 보장하지는 않습니다.

같은 frame은 보이는 `Working` 문구나 selection panel이 typed activity로 명시한 title status 위에서 peak grapheme 하나를 첫 글자부터 마지막 글자까지 이동한 뒤 다시 첫 글자로 순환시킬 수 있습니다. peak 양옆의 grapheme은 각각 최대 하나까지 중간 밝기의 trail style을 사용하며, label 경계에서는 반대편으로 이어지지 않고 잘립니다. 나머지 글자는 muted style을 사용합니다. `ActivityMotionFrame`의 공통 resolver 하나가 peak와 선택적인 좌우 trail index를 계산하고 shell chrome과 selection panel이 모두 이를 사용해야 합니다.

Muted, trail, peak, marker는 서로 다른 appearance role입니다. 내장 role은 터미널 기본 foreground 또는 palette-indexed color만 사용합니다. 고정 RGB는 foreground와 background를 함께 결정할 수 있는 향후 명시적 theme 설정으로 미룹니다.

Sheen은 style만 바꾸며 글자, 셀 폭, 행과 panel geometry, fitting 결과, 입력 동작, 중단 안내를 바꾸면 안 됩니다. activity가 아닌 title status는 정적이어야 하고 marker와 모든 보이는 sheen은 같은 elapsed sample을 사용합니다.

유효한 frame 하나만 가진 profile은 marker와 sheen 모션을 모두 끄고 timed redraw를 요청하지 않습니다. 보이는 sheen이 두 grapheme보다 짧아 phase가 바뀌어도 cell이 달라지지 않는 경우에도 redraw를 요청하지 않습니다. 하나의 committed appearance snapshot과 revision이 한 논리 frame의 marker cycle과 period를 함께 공급하며, frame 준비 중 replacement는 다음 완전한 frame부터 적용됩니다.
