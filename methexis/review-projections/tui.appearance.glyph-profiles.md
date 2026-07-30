---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.appearance.glyph-profiles
revision: sha256:fe3666c9aa6a778c41e9b2a6216bb8171b737895d107e95a3bd7710a54cf6a59
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:ace75a31acbc9957d92d649c82198369221eb52b70e6c9baf2c90d98b3f0adda
---
# Korean Review Projection

## Translation

초기 appearance vocabulary의 transcript marker는 user가 Rich `❯`(`U+276F`), ASCII `>`(`U+003E`)이고 assistant가 Rich `⏺`(`U+23FA`), ASCII `*`(`U+002A`)여야 합니다.

`Rich`는 현재 호환 기본값을 유지합니다. `Ascii`는 명시적인 session appearance candidate로만 선택합니다. 초기 구현은 `TERM`에서 profile을 추론하면 안 되며 color capability나 `NO_COLOR`도 glyph profile을 선택하면 안 됩니다.

candidate marker는 비어 있지 않은 extended grapheme cluster 정확히 하나여야 하고, 게시 전에 control, ANSI 내용, zero-width cluster를 거부해야 합니다. 측정은 appearance 전용 width table이 아니라 기존 `yo-unicode-17.0-narrow/v1` Surface width owner를 사용합니다. 허용된 모든 marker는 설정된 body indent 안에 들어가야 합니다.

Rich와 ASCII marker의 cell width가 같을 필요는 없습니다. 공통 indent 안에서 보정하여 두 profile 모두 user와 assistant 본문이 같은 설정 column에서 시작해야 합니다. 화면과 plain session output은 같은 committed snapshot에서 marker를 얻어야 합니다.
