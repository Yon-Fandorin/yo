---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.model.service-binding
revision: sha256:3cf709c45e94c810ae926047b3e90cd9025591f34a99b8d1914d57bf5e6b7cbc
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:3e9e879f567137cbfbe82febf0140b5e45d1c4d8e08c34d70ba03d15f2ad6247
---
# Korean Review Projection

## Translation

# 모델 서비스 바인딩 식별자

## Statement

모델 라우팅은 서로 구분되는 typed `ProviderId`, `AccountId`, `ModelId`를 사용해야 합니다. 각 식별자는 로컬 설정 안에서 안정적이어야 하며 선택적인 표시 이름은 식별자를 바꾸지 않고 변경할 수 있습니다. Provider는 하나의 모델 서비스 정책으로 모델을 제공하는 그룹이며 Agent Backend가 아닙니다. Account는 Provider 안에서 로컬 credential과 entitlement 범위 하나를 선택하고, 정확한 `ProviderId`와 `AccountId` 조합이 credential을 선택합니다. Model은 wire에 전송되는 정확한 모델 이름을 선택합니다.

이 좌표를 선택하는 외부 설정 key는 `provider`, `account`, `model`이어야 하며 내부 코드는 typed ID를 유지해야 합니다. 공개 catalog 설정은 명시적인 `api_dialect` 하나를 선택해야 하고 `connector` selector를 노출하면 안 됩니다. Startup 전에 closed runtime registry가 dialect를 허용된 built-in Model Connector 구현 정확히 하나로 결정론적으로 해석해야 합니다. Mapping이 없거나 모호하면 추측, probe 또는 caller가 고른 구현 이름을 허용하지 않고 설정 단계에서 실패해야 합니다.

완전한 effective model binding은 Provider, Account, Model, 해석된 Model Connector 구현, 명시적 API dialect, 정규화된 endpoint를 식별해야 합니다. 해석된 connector와 dialect는 허용된 정확한 조합이어야 합니다. 이 중 하나라도 바뀌면 다음 Turn 전에 새 backend binding epoch를 열어야 합니다. Durable attribution은 표시 이름이 아니라 안정적인 식별자를 사용하며 credential을 포함하면 안 됩니다. 해석된 connector identity는 runtime evidence이자 durable binding identity이며 중복된 공개 설정 선택지가 아닙니다.

`OpenAI-compatible`은 API family 이름일 뿐입니다. 설정과 runtime은 정확한 `api_dialect`를 사용해야 합니다. 처음 지원하는 값은 `openai-responses`와 `openai-chat-completions`입니다. API dialect는 request, response, streaming event, tool call, usage, terminal의 전체 문법을 정의합니다. HTTPS, SSE framing, 인증, deadline, decoding은 dialect 식별자 자체가 아니라 connector 구현 책임입니다. Chat Completions와 Responses를 하나의 dialect로 합치거나 probe와 fallback으로 선택하면 안 됩니다.

OpenRouter와 QwenCloud는 첫 first-class configured Provider입니다. 둘은 provider-neutral Yo-managed backend를 공유해야 합니다. 같은 dialect를 선택한 catalog entry는 그 dialect의 provider-neutral built-in connector로 해석되어야 하며 하나의 Provider가 서로 다른 허용 dialect로 서로 다른 Model을 제공할 수 있습니다. Provider별 endpoint, credential, Model, tokenizer profile 값은 catalog entry와 conformance evidence에 속하며 어느 Provider도 backend kind나 connector identity가 되어서는 안 됩니다.

Tenant 선택과 UI, account rotation과 failover는 미룹니다. 첫 구현에는 `TenantId`나 tenant field를 추가하지 않습니다. 다만 binding과 credential resolution은 주입 가능한 경계를 유지해 추후 caller-owned tenant scope가 Provider와 Account 조합을 선택하더라도 Provider, Account, Model, dialect, backend identity를 다시 정의하지 않게 합니다.

## Rationale

서비스 그룹, credential 범위, wire Model, dialect 문법, connector 구현을 분리하면 표시 이름이나 vendor 이름이 routing authority가 되는 일을 막을 수 있습니다. 현재의 일대일 connector mapping을 내부에서 파생하면 중복되거나 모순될 수 있는 공개 설정을 없애면서, 별도 검토된 선택 계약 이후 발전할 수 있는 내부 구현 경계를 유지합니다. 정확한 effective binding identity는 Provider, Model, dialect, endpoint, connector 구현 변경을 계속 감사 가능하게 합니다.
