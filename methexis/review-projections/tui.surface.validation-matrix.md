---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.surface.validation-matrix
revision: sha256:b50c6d872a02e25a95c7397bf8c46a1f32bbcdeb22d429c49832cffbb9e1bd1d
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:e8edeff170eb2ba51d5764e734b667cc2ec3f26a753d286fe41cab34466d7871
---
# Korean Review Projection

## Translation

실제 PTY에서 관찰한 출력이 terminal behavior의 권위여야 합니다. 초기 지원 matrix는 최신 macOS·Linux terminal, local tmux, SSH, SSH 안의 tmux를 포함해야 합니다. 결정론적 HTML fixture는 진단과 parity evidence이며 terminal check를 대체하지 않습니다.

환경 의존 실패는 결정론적 model·operation·projection 실패와 분리해 보고해야 합니다. 실행할 수 없는 matrix 항목은 조용히 통과시키지 말고 명시적으로 unverified 상태로 추적해야 합니다.

이 구분은 agentic TUI의 실제 환경을 다루면서 환경 불확실성과 재현 가능한 code defect를 분리합니다.
