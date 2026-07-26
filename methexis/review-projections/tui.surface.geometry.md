---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.surface.geometry
revision: sha256:41f9fe004f1a95d1d95b6810cd05408e5e356e17858ee3f7f0002164d2abff8f
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:3fccc0b24a0680d9c4f49ec4c5ec897cf52771bf86d4409dbe174e8c547b9450
---
# Korean Review Projection

## Translation

`Point`, `Size`, `Rect`는 `u16` 좌표와 크기를 사용해야 합니다. 기하 연산은 checked arithmetic을 사용하고, overflow 시 감싸거나 조용히 보정하지 말고 실패를 보고해야 합니다. 더 큰 문서·스크롤 위치는 viewport를 `Surface` 좌표로 사상하는 상위 모델이 소유합니다.

터미널 viewport는 제한되어 있지만 application document는 그렇지 않습니다. 두 영역을 분리하면 overflow가 명시적이고 rendering model이 간결해집니다.
