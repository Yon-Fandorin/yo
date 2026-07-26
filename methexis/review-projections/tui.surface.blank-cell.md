---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.surface.blank-cell
revision: sha256:ec8988e176bf90cbe93c8c0d19c547dbf20fe006e08d79fc89ceb7d052d7ba85
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:46300006b125718cf0b647f6f5b59554cc975577f977ea38da374dec43d9b2a5
---
# Korean Review Projection

## Translation

`Blank`는 grapheme 점유가 없고 하나의 resolved `Style`을 가진 명시적 cell state여야 합니다. 새로 만들거나 명시적으로 reset한 `Surface`는 terminal-default foreground와 background, attribute 없음의 `Blank`로 채워야 합니다.

clear operation은 명시적 resolved `Style`을 받아야 합니다. grapheme overwrite 과정에서 비워지는 모든 cell은 incoming write style을 사용해 `Blank`가 되어야 하며, 그 background와 attribute는 보존하되 이전 grapheme ownership은 남기면 안 됩니다.

명시적인 styled blank는 초기화, clear, diff, HTML parity를 관찰 가능하게 하고 ghost content와 무관한 default background 사이의 숨은 선택을 없앱니다.
