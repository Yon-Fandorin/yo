---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.runtime.command-event-boundary
revision: sha256:3fdc1fe8302195d1aac087bb723f14080a782f50076f25ae50bd0c7d8533c9b2
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:e8b9fe21c100f65edf6ebdba1ac5b6ed68130c9dc12141ecfb14fe89a76b7586
---
# Korean Review Projection

## Translation

# 에이전트 명령과 사건 경계

## 계약

프런트엔드는 세션이나 백엔드 내부를 직접 조작하지 않고 이름이 있는 타입화된 명령과 사건을 통해 `yo-core`와 상호작용해야 합니다. 명령과 사건은 세션을, 해당하는 경우 턴을 식별해야 합니다. 활동 생성·시작·갱신·응답을 포함하여 Activity나 요청 상관관계 대상이 적용되는 곳에서는 해당 Activity 식별자나 명시적인 요청 상관관계 식별자를 포함해야 합니다. `yo-core`가 실행 동작을 결정하고 의미 관찰을 내보내며, 프런트엔드는 입력 제스처와 표현을 결정합니다. 명령은 기존 타입화된 런타임 흐름에 실려 전달되는 에이전트 도메인 의도입니다.

초기 경계는 프로세스 내부 Rust 타입과 채널을 사용할 수 있습니다. Codex 전용 통신 이름을 yo 도메인 타입에 넣거나 실제 원격 소비자가 생기기 전에 원격 통신 프로토콜을 도입해서는 안 됩니다.

## 이유

하나의 의미 경계를 두면 TUI와 미래 GUI 클라이언트가 동작을 공유하면서 백엔드 프로토콜, 렌더링, 입력 정책을 각각 독립적으로 교체할 수 있습니다.
