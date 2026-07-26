---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.surface.bounded-view
revision: sha256:87ae8d0afee3a38ac35fe33cc9d7edfcbc96809236d6931e1fa22f7bd5fb9634
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:8c5d76f0d49d3c73ed3b11950aeb7fca6eec599aec9ac3b07f0e97354cee3642
---
# Korean Review Projection

## Translation

component는 할당된 `Rect`로 제한된 `SurfaceView`를 통해 렌더링해야 합니다. view는 최종 셀 상태를 읽고 제한된 검증 write operation을 사용할 수 있지만, mutable backing storage를 노출하면 안 됩니다.

초기 renderer에는 retained widget tree, layer, z-index를 넣지 않습니다. caller가 component render 호출 순서로 composition을 결정합니다.

좁은 view는 component가 다른 영역을 손상시키는 일을 막고, 명시적 draw order는 실제 overlap 요구가 생길 때까지 예측 가능한 구성을 제공합니다.
