---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.session.continuation-lineage
revision: sha256:1fabb0dbaacd6ab08321fc4b888175f44e57b9bdd54f8c858951f13419a796e5
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:c84f2590d045e06542ce79747cc4e4cd8f4ca5db5e2f85e19d9dce4b76251fc6
---
# Korean Review Projection

## Translation

# 세션 재개와 계보

## 결정

Yo Session은 하나의 사용자 작업을 나타내는 UUIDv7 영속 정체성입니다. Backend, locator, transport, model이 바뀌어도 같은 Session을 유지하고 실행 정보는 순서와 버전이 있는 binding epoch로 기록합니다. 전환은 이전 epoch를 닫고 새 epoch를 열며 모든 anchor가 자기 epoch를 식별합니다. Journal 소비자는 epoch 경계를 보존합니다.

사용자가 의도한 fork만 새 Session을 만듭니다. Parent와 source anchor 또는 anchor 부재를 기록합니다. Anchor가 없는 빈 child도 fork입니다. Source anchor가 있는 fork는 검증된 backend-native fork 또는 replacement binding과 같은 exact replay 및 명시적으로 승인된 lossy handoff 규칙으로 첫 binding을 만듭니다.

기록 읽기와 실행 재개는 분리합니다. Continuation Anchor는 수락된 request, 안정적인 결과, 완전히 커밋된 semantic Journal 경계, backend binding과 locator를 식별합니다. Request Audit의 payload, header, revision, attempt 상세는 anchor 구성과 검증에 필요하지 않습니다. Resume은 최신 durable anchor만 선택하며 이후 history가 없는 이전 locator로 fallback하지 않습니다. 불완전한 suffix는 자동 입력이 아닙니다.

Anchor가 없으면 Session을 읽기 전용으로 열고 명시적으로 확인한 빈 fork만 제안합니다. Uncommitted suffix를 replay하거나 재전송하지 않으며 recovery snapshot만으로 anchor를 만들지 않습니다.

Native Resume은 locator와 backend identity를 검증하고 성공하면 같은 binding을 계속 사용합니다. 실패했지만 exact semantic replay가 가능하면 같은 Yo Session에 replacement binding을 만들 수 있습니다. Role, 순서, 정확한 committed text, tool call-result 관계와 adapter에 필요한 semantic record를 보존하지만 provider cache, hidden state, 동일한 미래 출력은 보장하지 않습니다. 전환, backend/model identity, replay boundary, cache loss를 기록합니다.

Lossy handoff만 가능하면 누락 context를 설명하고 한 번 확인받습니다. 승인 후 같은 Session에 replacement binding을 만들 수 있지만 visible context-loss boundary와 원래 durable history를 보존합니다. 몰래 손실 전환하거나 불확실한 request를 재전송하거나 native resume이라고 표현하지 않습니다.

## 이유

Provider 변경만으로 사용자 작업 정체성을 바꾸지 않으면서도 epoch와 context-loss 경계로 실제 연속성의 강도를 정직하게 기록하기 위함입니다.
