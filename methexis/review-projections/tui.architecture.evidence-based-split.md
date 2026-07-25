---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.architecture.evidence-based-split
revision: sha256:0787fb2d64d3d16201752a02130ea45f9287734f37b6bf10f0269f6b239f8794
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:ecd38022af5418e78b7ac077a39c567e5ad93f4fd6b65ffa00b9fe8a458f7548
---
# Korean Review Projection

## Translation

독립적인 프로덕션 소비자가 공유 계약, 의존성 경계, 릴리스 주기를 입증할 때까지 프로덕션 코드는 하나의 `yo-tui` 크레이트에 유지해야 합니다. 향후 Tauri 애플리케이션은 안정된 도메인 의미의 추출을 정당화할 수 있지만, 터미널 전용 UI 구조의 추출을 정당화하지는 않습니다.

두 번째 소비자가 생기기 전에 분리하면 추측으로 공개 경계를 만들고, 독립적인 가치를 입증하지 못한 채 이후 변경을 더 어렵게 만듭니다.
