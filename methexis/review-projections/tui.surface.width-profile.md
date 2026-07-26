---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.surface.width-profile
revision: sha256:6f83deba02e9cce6473c947191acca41e160fe9a78a3a2b9e1646ecd5aac0883
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:4de4ff7501192720e560d63dfcf00c6835356c92887635f87e9c16f139ffcb5e
---
# Korean Review Projection

## Translation

터미널과 HTML projection은 Unicode 17.0 데이터 기반의 정확한 profile `yo-unicode-17.0-narrow/v1`을 함께 사용해야 합니다. 하나의 완전한 extended grapheme cluster만 받아 너비 1 또는 2를 반환합니다.

Unicode Emoji 17.0 `emoji-variation-sequences.txt`에 등재된 표준 text presentation sequence의 `VS15`는 scalar의 default emoji presentation보다 우선해 non-emoji 규칙을 따릅니다. 그 외 Unicode Emoji 17.0의 RGI emoji sequence, `Emoji_Presentation=Yes` scalar, 또는 같은 파일에 등재된 표준 emoji presentation sequence의 `VS16`을 포함한 cluster는 너비 2입니다. 나머지는 combining mark, variation selector, ZWJ, default-ignorable scalar가 0, East Asian Width `W`·`F`가 2, `A`·`H`·`Na`·`N`이 1을 기여하며 cluster 너비는 최대 기여값입니다. 표준 variation sequence에 속하지 않는 `VS15`나 `VS16`은 presentation 효과가 없고 0을 기여합니다.

모두 0인 cluster는 `ZeroWidth`로 거부합니다. newline과 tab은 text layout이 처리하고 다른 control character도 셀 변경 전에 거부합니다. 여러 grapheme cluster가 든 입력도 거부합니다.

추후 terminal capability detection으로 다른 profile을 선택할 수 있지만 identity를 기록해야 합니다. profile 변경은 완성 프레임을 무효화하고 full redraw를 강제하며 기존 셀을 조용히 재해석하면 안 됩니다.

segmentation, Unicode 데이터, emoji 처리와 width resolution을 고정하면 conforming adapter 사이의 occupancy 차이를 막을 수 있습니다.
