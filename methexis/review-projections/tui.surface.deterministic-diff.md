---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.surface.deterministic-diff
revision: sha256:269a7815cb3c6b213295b70da7c26ddc2dded7a776bfbe12353d5b2ebff41e4c
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:d80561b2c3a4d6dabd59cc3351cf90a7364dd317e1ab16c14cc92b4b479fe5d3
---
# Korean Review Projection

## Translation

diff는 immutable한 이전·현재 완성 `Surface`를 비교하고, 변경된 row span을 row와 column 오름차순으로 내보내야 합니다. 결과는 grapheme 경계를 보존하고 adapter가 component를 다시 보지 않고 렌더링할 만큼 resolved cell state를 포함해야 합니다.

첫 구현은 전체 프레임을 비교합니다. dirty-region tracking은 측정 근거가 생긴 뒤 같은 결정론적 결과를 보존하는 경우에만 도입할 수 있습니다.

완성 프레임 비교는 단순한 correctness oracle과 안정된 fixture를 제공합니다. 최적화는 이 oracle에 대해 검증해야 하며 새로운 semantic을 정의하면 안 됩니다.
