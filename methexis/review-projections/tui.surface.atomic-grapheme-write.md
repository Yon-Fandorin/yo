---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.surface.atomic-grapheme-write
revision: sha256:c1cdcafa4c92e4e590431b36b8afa3cdeace0e2a9b3355bc544bb76580eaac02
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:e971d0faf5561f4f7a793f5ba228f512499a42cc6edc3ef213b0629a66ceed9d
---
# Korean Review Projection

## Translation

grapheme write는 점유하는 모든 물리 셀을 함께 갱신하거나 아무 변경도 하지 않아야 합니다. 남은 view 경계 안에 전체 grapheme이 들어가지 않으면 `Clipped`를 반환하고 기존 상태를 그대로 둬야 합니다.

primitive는 주변 셀을 밀거나 당기지 않는 물리 셀 overwrite와 기존 grapheme footprint 전체 정리를 소유합니다. wrapping, ellipsis, 논리 텍스트 시퀀스의 insertion, deletion, replacement와 문자 폭 변경 뒤의 reflow는 `SurfaceView` 위의 text model과 layout이 소유합니다. 그 계층이 최종 위치를 다시 계산하고 렌더링하므로 `가B`를 논리적으로 `AB`로 바꾸면 지워진 continuation이 빈 간격으로 보이지 않고 `A`와 `B`가 붙어야 합니다.

원자적 실패는 고립된 continuation과 숨은 부분 쓰기를 막습니다. reflow를 cell primitive 밖에 두면 local write가 인접한 border나 다른 component를 잘못 이동시키는 일도 방지합니다.
