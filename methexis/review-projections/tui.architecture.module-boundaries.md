---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.architecture.module-boundaries
revision: sha256:4c2604b602190817b68ceccf6f5e726fadc89b5fb32875dedcc17d53bfa1533e
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:ea589e50c06c8e28faee0845a0f233781f504d25a3055c4658197919f4192a6c
---
# Korean Review Projection

## Translation

`yo-tui` 내부 의존성은 터미널에 독립적인 기반 모듈을 향해야 합니다. 컴포넌트는 동일한 입력에서 동일한 결과가 나오는 구조화 출력을 만들고, 터미널 I/O를 수행하거나 원시 ANSI 제어 바이트를 출력해서는 안 됩니다.

이 안쪽 방향의 의존성 덕분에 터미널 adapter와 문서 adapter가 동일한 구조화 UI 모델을 소비할 수 있으며, 어느 adapter도 컴포넌트의 의미를 소유하지 않습니다.
