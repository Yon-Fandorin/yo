---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.runtime.command-event-boundary
revision: sha256:3f107762677c3569560bed6c9e97e3f89da0b850d54f0147cf83c9c6882cb334
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:414b1a818e32e8dbb2a5af1b6125f11466476280a7db5db9bb25a47820ce74df
---
# Korean Review Projection

## Translation

프런트엔드는 session이나 backend 내부를 직접 조작하지 않고, 이름이 있는 타입화된 command와 event를 통해 `yo-core`와 상호작용해야 합니다. command와 event는 대상 Session을 식별하고, 해당되는 경우 Turn도 식별해야 합니다. Activity의 생성, 시작, 갱신, 응답 경로를 포함하여 Activity 또는 request correlation 대상이 해당될 때마다 command나 event는 그 Activity identity 또는 명시적인 request correlation identity를 전달해야 합니다. `yo-core`는 실행 동작을 결정하고 도메인 의미론적 관찰 결과를 내보내며, 프런트엔드는 입력 제스처와 표현 방법을 결정합니다. command는 기존 typed runtime flow가 운반하는 에이전트 도메인의 intent입니다.

초기 경계는 같은 프로세스 안의 Rust 타입과 channel을 사용할 수 있습니다. Codex 전용 wire 이름을 yo 도메인 타입에 넣어서는 안 되며, 실제 원격 소비자가 생기기 전에는 원격 wire protocol을 도입해서는 안 됩니다.

하나의 의미 경계로 TUI와 미래 GUI가 동작을 공유하면서 provider protocol, rendering, input policy를 서로 독립적으로 교체할 수 있습니다.
