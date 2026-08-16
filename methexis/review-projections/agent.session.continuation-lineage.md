---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.session.continuation-lineage
revision: sha256:d6dfeab7515fe4bd9e99dcd3bff00dec90205d040f265bcdc819cfe437f1ad9a
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:eebb0528bc886b9c689fd978a73543b2fd7e3c7fdbc9726c8ff4fb29f62e4e97
---
# Korean Review Projection

## Translation

# 세션 재개와 계보

## 결정

Yo Session은 하나의 사용자 작업을 나타내는 UUIDv7 영속 정체성입니다. Backend, locator, transport, model이 바뀌어도 같은 Session을 유지하고 실행 정보는 순서와 버전이 있는 binding epoch로 기록합니다. 전환은 이전 epoch를 닫고 새 epoch를 열며 모든 anchor가 자기 epoch를 식별합니다. Journal 소비자는 epoch 경계를 보존해야 하며 하나의 binding이 epoch 전환을 가로질렀던 것처럼 backend state를 replay, summarize, attribute하면 안 됩니다.

사용자가 의도한 fork만 새 Session을 만듭니다. Parent와 source anchor 또는 anchor 부재를 기록합니다. Anchor가 없는 빈 child도 fork입니다. Source anchor가 있는 fork는 검증된 backend-native fork 또는 replacement binding과 같은 exact replay 및 명시적으로 승인된 lossy handoff 규칙으로 첫 binding을 만듭니다.

기록 읽기와 실행 재개는 분리합니다. Continuation Anchor는 수락된 backend request, 그와 correlated된 안정적인 resumable outcome, 완전히 커밋된 semantic Journal 경계, 계속하는 데 필요한 versioned backend binding과 locator를 식별해야 합니다. 이 identity와 locator는 bounded Session Journal correlation data이며 선택적인 Request Audit 상세가 아닙니다. Request Audit의 payload, header, revision, attempt 상세는 anchor 구성과 검증에 필요하지 않습니다. Resume은 최신 durable anchor만 선택하며 이후 history가 없는 이전 locator로 fallback하지 않습니다. 불완전하거나 수락되지 않았거나 commit되지 않은 suffix는 diagnostic evidence로만 남고 자동 continuation input이 되면 안 됩니다.

Anchor가 없으면 Session을 읽기 전용으로 열고 명시적으로 확인한 빈 fork만 제안합니다. Uncommitted suffix를 replay하거나 재전송하지 않습니다. Recovery snapshot은 durable publication이 완료되고 모든 Anchor 조건을 만족한 뒤에만 이후 Continuation Anchor를 뒷받침할 수 있으며 snapshot만으로는 Anchor를 만들 수 없습니다.

Native Resume은 locator와 backend identity를 검증하고 성공하면 같은 binding을 계속 사용합니다. 실패했지만 exact semantic replay가 가능하면 같은 Yo Session에 replacement binding을 만들 수 있습니다. Role, 순서, 정확한 committed text, tool call-result 관계와 target adapter에 필요한 모든 backend-visible semantic record를 보존하지만 provider cache나 동일한 미래 출력은 보장하지 않습니다. Replay profile이 provider-private replay를 선언한 binding은 같은 Anchor와 atomic replay commit을 통해 모든 필수 private item도 lossless하게 보존해야 합니다. 이 item은 resumed binding이 같은 정확한 binding identity와 replay profile을 가질 때만 사용할 수 있고 generic history로 projection하면 안 됩니다. Source Anchor가 private item을 포함하는데 target binding이나 profile이 그 정확한 item을 소비할 수 없다면 전환은 `exact_replay`가 아닙니다. 독립적으로 검토된 lossless conversion 계약이 없다면 별도로 승인된 `lossy_handoff` 경로가 필요합니다. Target 자체가 private state를 요구하지 않더라도 K3 effort, K2.7 Code ModelId나 speed tier, endpoint, connector, replay profile 또는 schema가 바뀌는 경우에도 같은 규칙을 적용합니다. Source state가 missing, wrong-epoch, unbounded, wrong-schema인 경우에도 조용히 빼는 대신 exact replay를 사용할 수 없습니다. 전환, backend/model identity, replay boundary, private-replay availability, cache loss를 기록합니다.

Lossy handoff만 가능하면 Yo는 먼저 저장된 Session을 read-only로 열고 누락되거나 변형된 context를 설명한 뒤 계속하기 전에 한 번 확인받아야 합니다. 명시적 승인 후 같은 Session에 replacement binding을 만들 수 있지만 Journal은 visible context-loss boundary를 기록하고 원래 durable history를 그대로 보존해야 합니다. 몰래 손실 전환하거나 불확실한 request를 재전송하거나 replacement binding을 native resume이라고 표현하면 안 됩니다. Provider-private replay의 내용은 이 안내에서도 숨겨야 하며 operator에게는 schema, presence, byte count, target의 보존 가능 여부만 보여줍니다.


각 backend binding은 versioned continuation strategy 하나를 명시합니다. exact_replay는 local_client 또는 managed_server executor를 가지며 backend_managed_state는 replay executor를 가지지 않습니다. Strategy는 backend kind, Provider, API dialect, model name에서 추론하지 않고 binding transition mode와도 구분합니다. 예를 들어 새로 열린 binding은 `exact_replay` transition으로 seed된 뒤 두 선언된 strategy 중 하나를 사용할 수 있습니다. Exact-replay binding은 complete effective binding의 `replay_profile`을 binding evidence에 포함해야 합니다. `semantic-only/v1`은 private replay를 금지하고 `kimi-private-local-plaintext/v1`은 `kimi.assistant-message/v1alpha1`을 선언합니다. Profile은 binding identity와 epoch freshness의 일부이며 ModelId에서 추론하면 안 됩니다. Format-compatibility 계약이 정한 정확한 legacy omission은 오직 `semantic-only/v1`로 decode합니다. 두 exact replay executor는 같은 semantic replay contract, validation, Anchor boundary를 사용하며 validated prefix를 읽고 다음 request를 조립하는 위치만 다릅니다. local_client는 local Session Repository에서 복원합니다. managed_server는 향후 Yo-managed Session service를 위한 예약 capability이며 remote repository identity, replay boundary, content 및 contract digest, binding epoch, availability, retention을 검증하는 reviewed implementation 전에는 광고하지 않습니다. backend_managed_state에서는 Yo가 transcript, semantic event, correlation, locator를 보관하고 backend가 model-visible conversation state를 소유합니다. 이 Anchor는 Yo replay delta를 참조하거나 보유한다고 주장하지 않습니다.

## 이유

Provider 변경만으로 사용자 작업 정체성을 바꾸지 않으면서도 epoch와 context-loss 경계로 실제 연속성의 강도를 정직하게 기록하기 위함입니다.
