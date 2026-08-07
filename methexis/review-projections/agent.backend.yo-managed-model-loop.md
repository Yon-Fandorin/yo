---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.backend.yo-managed-model-loop
revision: sha256:e8acc4fdfb2465b2c5bb8e0d4fbca46b4ef2f4227e8df13abdf2bc325aac6285
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:83d9a83e12d70f19e1bdeb5bddef845bc00f6fb24a16e998eab2be37de757f44
---
# Korean Review Projection

## Translation

# Yo-managed model과 tool loop

## 계약

첫 Yo-managed Agent Backend는 기존 AgentBackend semantic port를 구현하면서 yo-core 안에서 model loop, tool execution coordination, model-visible context를 소유합니다. Model Connector는 remote request와 stream protocol만 소유하며 yo-cli, frontend, connector는 agent-loop owner가 될 수 없습니다.

Accepted Turn마다 backend는 committed semantic Session history와 새 user input을 selected model protocol로 project합니다. Text delta는 기존 message segmentation과 terminal-seal 경로를 통해 ModelWork Activity가 됩니다. Model function call은 wire call identity, function name, 누적 argument bytes를 정확히 보존합니다. Validation이 거부해도 correlated Tool Activity를 만들며 invalid JSON, schema mismatch, unknown 또는 duplicate identity, unavailable tool, argument bound failure는 effect 없이 typed validation failure로 Activity를 끝냅니다. Approval, admission, dispatch 전에 validation이 성공해야 하며 approval과 execution은 frozen registry, admission policy, execution-host boundary를 사용합니다. Model service가 local workspace tool을 직접 실행하지 않습니다.

Backend는 function call과 exact tool outcome을 record한 뒤 대응 function-call output을 다음 model request에 보냅니다. 한 response의 여러 call은 tool scheduler가 approval과 mutable resource lease의 독립성을 증명할 때만 concurrent execution할 수 있고 아니면 model order로 실행합니다. 완료 순서와 관계없이 result는 stable call order로 반환합니다. Missing, duplicate, mis-correlated call이나 result는 Turn을 실패시킵니다.

Loop는 final assistant message, accepted cancellation, bounded model-round limit, typed failure 중 하나까지 model response, local tool execution, tool-result submission을 반복합니다. Session은 active Turn 하나 제한을 유지합니다. Cancellation은 connector work를 신속히 중단하고 새 tool execution을 막고 active Activity를 interrupted로 seal한 뒤 connector와 tool cleanup을 실행합니다.

Provider response ID, cache handle, conversation ID는 diagnostic correlation으로 보존할 수 있지만 유일한 continuation locator가 될 수 없습니다. Yo-managed binding은 provider-native resume가 아니라 exact semantic replay를 광고합니다. Executable continuation은 Session Journal에서 최신 durable Continuation Anchor가 지시한 model-visible semantic boundary를 재구성하고 endpoint, protocol, Provider, Account, Model, connector identity가 바뀌면 새 binding epoch를 엽니다. Anchor 뒤의 committed mid-Turn function call, tool result, partial stream 또는 다른 suffix는 diagnostic으로만 남고 automatic continuation input이 되지 않습니다. Durable Anchor가 없으면 replay input을 만들지 않고 continuation contract의 read-only fallback을 따릅니다. Exact replay는 message role과 order, exact visible text, function-call과 tool-result relation, recorded system 및 tool contract를 보존합니다. Hidden reasoning과 provider cache state는 replay claim이 아닙니다.

Partial model stream, uncommitted tool result, uncertain request, failed final response를 Continuation Anchor가 덮으면 안 됩니다. Usage와 exact effective binding은 같은 Yo Session 안에서 모델이 바뀌어도 그것을 생성한 model response에 귀속합니다.

## 이유

Loop를 yo-core가 소유하면 frontend-independent Session contract를 유지하면서 genuinely native backend를 제공합니다. Durable Anchor 기준의 exact semantic replay는 partial Turn이나 uncertain side effect를 continuation으로 오인하지 않으면서 provider의 temporary response retention과 분리합니다.
