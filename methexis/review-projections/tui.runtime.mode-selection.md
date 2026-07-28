---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.runtime.mode-selection
revision: sha256:da04bd60dc60009bb82238da9fe97af2dd5c6afa772b653e6cac594147d3ff1d
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:937593bc47557b3516c187531f900ac4a6f38082bce906a77ca91b125a2228a0
---
# Korean Review Projection

## Translation

live `yo` CLI는 Inline과 Fullscreen을 명시적으로 선택할 수 있어야 합니다. 두 선택은 같은 application state와 interaction 결과를 사용하되, 각 presenter의 terminal 소유권과 복구 계약은 구분해서 유지합니다.

Auto 알고리즘이 별도로 승인되기 전까지 mode option 없이 `yo`를 실행하면 현재 배포된 Inline 동작을 유지합니다. 명시적인 Inline 또는 Fullscreen 선택은 terminal state를 획득하기 전에 이 호환 기본값보다 우선해야 합니다. 검수되지 않은 환경 heuristic을 Auto 동작으로 공개해서는 안 됩니다.

option이 없을 때의 Inline 동작은 장기 제품 기본값이 아니라 임시 호환 정책입니다. 추후 Inline, Fullscreen, Auto 중 무엇을 기본으로 할지는 비슷한 agent 도구 조사와 terminal, tmux, SSH, 원격 tmux 검증 자료를 바탕으로 별도 결정합니다.

이 방식은 기존 실행을 깨뜨리지 않으면서 두 presenter를 실제로 사용하고 검증할 수 있게 합니다. terminal 진입 전에 mode를 결정하므로 한 mode를 일부 획득한 뒤 다른 mode로 fallback하는 위험도 피합니다.
