---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.model.service-binding
revision: sha256:f34076fe173c01fc196a04efe55a3298567038cd9ed7993affb67d0178e48ef3
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:b78d0e9f96391ae574896ab01b99450ecaeb9e341950bdb38eba8c3574b62614
---
# Korean Review Projection

## Translation

# 모델 서비스 binding identity

## 계약

모델 routing은 서로 다른 typed ProviderId, AccountId, ModelId를 사용합니다. 각 ID는 local configuration 안에서 안정적이며 선택적인 display name은 identity를 바꾸지 않고 변경할 수 있습니다. Provider는 하나의 model-service 정책으로 모델을 제공하는 그룹이지 Agent Backend가 아닙니다. Account는 Provider 안에서 하나의 local credential과 entitlement scope를 고르고, Model은 wire로 보낼 정확한 모델 이름을 고릅니다.

외부 설정에서 account를 고르는 key는 account이고 내부 코드는 AccountId를 운반합니다. 완전한 effective binding은 Provider, Account, Model, Model Connector, 명시적인 API protocol, 정규화한 endpoint를 식별합니다. 이 필드 중 하나라도 바뀌면 다음 Turn 전에 새 backend binding epoch를 열어야 합니다. Durable attribution은 display name이 아니라 안정적인 ID를 사용하며 credential을 포함하지 않습니다.

OpenAI-compatible은 protocol family 이름일 뿐입니다. 설정과 runtime은 정확한 api_protocol을 사용하며 첫 값은 openai-responses입니다. Chat Completions와 Responses를 하나의 protocol 값으로 합치거나 probing과 fallback으로 고르면 안 됩니다.

Tenant 선택, tenant UI, account rotation과 failover는 미룹니다. 첫 구현에 TenantId나 tenant field를 추가하지 않습니다. 다만 binding과 credential resolution은 injected boundary로 유지해 나중에 caller-owned tenant scope가 Account를 고를 수 있게 하며 Provider, Account, Model, backend identity는 다시 정의하지 않습니다.

## 이유

Service, credential scope, model, protocol을 분리하면 display label이나 vendor name이 routing authority가 되는 것을 막습니다. Exact binding identity는 tenant behavior를 미리 구현하지 않고도 모델 변경과 continuation을 감사 가능하게 합니다.
