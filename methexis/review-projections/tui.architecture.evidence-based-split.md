---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.architecture.evidence-based-split
revision: sha256:b8022eaed3bf1313bb75a4e7c15bb3aa698550415f3845e9daa0e9544c8a2b42
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:7fb51fcc23cd85337a9718bcfabdf06ce8f8ad7b8641d04b0995d4494267f3cc
---
# Korean Review Projection

## Translation

`yo-tui`는 독립 소비자가 공유 계약, 의존성 경계, 릴리스 주기를 입증할 때까지 단일 프로덕션 TUI 라이브러리로 유지합니다. `yo-cli` 같은 제품 진입 패키지는 이 라이브러리를 조합하고 프로세스 전체 정책을 소유할 수 있으며, 이는 공유 TUI 또는 도메인 내부 구조를 추측으로 분리하는 것이 아닙니다. 향후 Tauri 애플리케이션은 안정된 공유 의미의 추출을 정당화할 수 있지만 터미널 전용 UI 구조의 추출을 정당화하지는 않습니다.

두 번째 소비자가 생기기 전에 공유 라이브러리를 나누면 추측으로 공개 경계를 만들고 이후 변경을 어렵게 합니다. 반면 실행 가능한 제품 진입점은 공유 라이브러리 추출과 다른 필수 조합 경계입니다.
