---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.architecture.module-boundaries
revision: sha256:95366906f598718e308296d25ff8e765ba6fc8dd602eff8ce4eaea95eb249ffb
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:d034a404267d2cf6c46887f7dab071ccd2342f10052583d5544cae0a25536d5a
---
# Korean Review Projection

## Translation

`yo-tui` 내부 의존성은 터미널 독립 기반 모듈을 향해야 합니다. 컴포넌트는 결정론적 구조화 출력을 만들고 터미널 I/O나 원시 ANSI 제어 바이트를 직접 출력하면 안 됩니다. 터미널 adapter는 TTY와 terminal-output 연산을 소유합니다. 애플리케이션 진입 host는 Unix signal 설치와 replay를 포함한 프로세스 전체 lifecycle 정책을 소유하고 반복 가능한 UI session에는 typed control observation만 제공합니다.

이 의존성 방향은 terminal/documentation adapter가 같은 UI 모델을 소비하면서 컴포넌트 의미를 소유하지 않게 합니다. 프로세스 정책을 제품 진입 host에 두면 UI 라이브러리가 미래 GUI나 다른 frontend의 lifecycle root가 되는 것을 방지합니다.
