---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.surface.resolved-style
revision: sha256:7210c4f0cdb9a5f7382c0e7edb7ea24d40b42181259854d2e4ae558b284ac33e
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:669546e637a6d52ae021943749052764c6bb15d0fcc157fd2b6113b18a90f501
---
# Korean Review Projection

## Translation

모든 물리 셀은 최종 결정된 foreground, background, attribute `Style`을 inline으로 저장해야 합니다. semantic role, theme lookup, style composition은 `Surface`에 쓰기 전에 끝나야 합니다.

초기 모델에는 `StyleId` 간접 참조를 넣지 않습니다. 메모리나 비교 성능의 실질적 이점이 측정되고 adapter가 보는 resolved semantic을 보존할 때만 교체할 수 있습니다.

resolved style은 diff와 projection에 하나의 명확한 비교값을 주며, inline 저장은 초기 정책 면적을 가장 작게 유지합니다.
