---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.surface.intersecting-overwrite
revision: sha256:f3a763bf9e406f42fc674a22fbe37e1585074bffb66436854553f48408d2aa0f
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:30c6933872f32f19f8a47f473d10369ffcce5adfcdbf692d5d6790254d2a81b2
---
# Korean Review Projection

## Translation

grapheme write는 변경 전에 새 footprint와, 그 영역에 겹치는 기존 leader 또는 continuation의 완전한 footprint를 계산해야 합니다. atomic mutation region은 이들의 합집합입니다.

새 footprint나 겹친 기존 footprint가 현재 `SurfaceView` 경계를 넘으면 `Clipped`를 반환하고 아무것도 바꾸지 않아야 합니다. 그렇지 않으면 전체 mutation region을 incoming resolved style의 `Blank`로 바꾼 뒤 같은 style로 새 leader와 continuation을 하나의 atomic change로 써야 합니다.

이 규칙은 더 좁은 grapheme이 넓은 leader를 대체하거나 continuation 위치에서 쓰기를 시작할 때도 적용됩니다.

기존 footprint 전체를 닫으면 orphan cell과 ghost text를 막고, view-bound check는 component isolation을 보존합니다.
