---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.model.service-binding
revision: sha256:fd6716219700886190c1370d26cd8c0d22d5a8e9988cb47f361d6948c7f450c2
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:c99dcb4ec258e439b6f54d4b09955f9aabf36e94c52b04eb66ca6887a9336a65
---
# Korean Review Projection

## Translation

# Model service binding identity

## 규칙

Model routing은 서로 구분되는 typed `ProviderId`, `AccountId`, `ModelId`를 사용해야 합니다. 각 identity는 로컬 설정에서 안정적이어야 하고 optional 표시 이름은 identity를 바꾸지 않고 변경할 수 있습니다. Provider는 하나의 model-service 정책으로 모델을 제공하는 그룹이며 Agent Backend가 아닙니다. Account는 한 Provider 안에서 로컬 credential과 entitlement scope 하나를 선택합니다. 정확한 `ProviderId`와 `AccountId` 조합이 그 credential을 선택합니다. Model은 wire로 보내는 정확한 모델 이름을 선택합니다.

이 좌표를 선택하는 외부 설정 key는 `provider`, `account`, `model`이어야 하며 내부 코드는 typed ID를 유지합니다. 완전한 effective model binding은 Provider, Account, Model, Model Connector, 명시적 API dialect, 정규화된 endpoint를 식별해야 합니다. 이 중 하나라도 바뀌면 다음 Turn 전에 새 backend binding epoch를 열어야 합니다. Durable attribution은 표시 이름이 아니라 안정적인 identity를 사용하며 credential을 포함하지 않습니다.

`OpenAI-compatible`은 API family 이름일 뿐입니다. 설정과 runtime은 정확한 `api_dialect`를 사용해야 하며 첫 지원 값은 `openai-responses`입니다. API dialect는 request, response, streaming event, tool call, usage, terminal의 전체 문법을 정의합니다. HTTPS, SSE framing, 인증, deadline, decoding은 dialect identity 자체가 아니라 connector 구현 책임입니다. Chat Completions와 Responses를 하나의 dialect로 합치거나 probe/fallback으로 선택하면 안 됩니다.

OpenRouter와 QwenCloud는 첫 first-class configured Provider입니다. 둘은 provider-neutral Yo-managed backend와 해당 dialect connector를 공유해야 합니다. Provider별 endpoint, credential, Model, tokenizer profile 값은 catalog entry와 conformance evidence에 속하며 어느 Provider도 backend kind나 connector identity가 되어서는 안 됩니다.

Tenant 선택과 UI, account rotation/failover는 미룹니다. 첫 구현에는 `TenantId`나 tenant field를 추가하지 않습니다. 다만 binding과 credential resolution은 주입 가능한 경계를 유지해 추후 caller-owned tenant scope가 Provider와 Account 조합을 선택하더라도 Provider, Account, Model, dialect, backend identity를 다시 정의하지 않게 합니다.

## 이유

Service group, credential scope, wire Model, dialect 문법, connector 구현을 분리하면 표시 이름이나 vendor 이름이 routing authority가 되는 일을 막을 수 있습니다. 정확한 binding identity는 여러 OpenAI-compatible Provider가 하나의 backend architecture를 공유하면서도 Provider와 Model 변경을 감사 가능하게 합니다.
