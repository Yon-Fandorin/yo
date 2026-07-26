---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.surface.grapheme-cells
revision: sha256:f0c1a62e8e1121618003f8b5c264fc77945afb7bec087037813ce2bbba6d72ff
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:a9dbcce7782ec752e2ad484ed299c464dc180736adc9a3e9a6dcecf56317a997
---
# Korean Review Projection

## Translation

렌더링된 grapheme은 전체 문자열과 표시 너비를 소유하는 leader 셀을 정확히 하나 가져야 합니다. 뒤에서 점유되는 각 셀은 leader까지의 0이 아닌 뒤쪽 거리를 담은 continuation이어야 합니다. leader와 continuation 모두 자기 물리 위치의 최종 resolved `Style`을 가져야 합니다.

변경 후에는 고립된 continuation이 없고 leader가 `Surface` 밖의 셀을 차지하지 않는 invariant가 유지되어야 합니다.

명시적 점유 모델은 wide character overwrite와 diff를 결정론적으로 만들고, 상대 back-reference는 문자열을 중복하지 않고 소유자를 찾게 합니다.
