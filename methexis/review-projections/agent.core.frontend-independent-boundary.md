---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.core.frontend-independent-boundary
revision: sha256:6102a5386dfcb7d1268cb73435c88fd558a5cd273963872a0aed9d9ef3e61b0a
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:ae06786530005694334927c5cefdcb4ee90786a44a4d8b4fd25a7e1201bf36e0
---
# Korean Review Projection

## Translation

# 프런트엔드 독립 에이전트 코어

## 계약

공유 에이전트 엔진의 이름은 `yo-core`여야 하며, 프런트엔드와 무관한 에이전트 실행 의미만 소유해야 합니다. `yo-tui`는 UI 동작을 소유하고, `yo-cli`는 제품 진입점, 프로세스 전역 lifecycle policy, top-level wiring, concrete Backend와 Model Connector composition을 소유합니다. 미래 GUI는 `yo-tui`에 의존하지 않고 `yo-core`를 재사용해야 합니다.

여러 곳에서 사용된다는 이유만으로 코드를 `yo-core`에 넣어서는 안 되며, 에이전트 실행 의미를 표현할 때만 이 크레이트에 속합니다. 초기 monolith는 semantic boundary가 안정화되는 동안 Session, command, event, configuration, Backend, Model Connector 관심사를 함께 유지했습니다. 이후 crate 분리는 공유된다는 이유만으로 코드를 추출하는 것이 아니라 독립적으로 변하는 구체적인 architecture boundary를 식별해야 합니다.

승인된 Backend 분리는 provider-neutral `yo-backend` foundation, `yo-core`의 `AgentBackend` semantic specialization, flat concrete crate인 `yo-backend-managed`, `yo-backend-delegated-codex`, `yo-backend-delegated-grok`으로 구성됩니다. Foundation은 generic evidence와 replay type을 유지하되 이를 해석하지 않는 bounded versioned opaque provider-private envelope만 포함합니다. Concrete backend는 foundation과 `yo-core`에 의존할 수 있지만 `yo-core`와 foundation은 concrete backend에 의존해서는 안 됩니다.

승인된 Model Connector 분리는 provider-neutral Connector port와 공통 Connector semantic request, observation, failure, cancellation, complete-binding type만 `yo-core`에 남깁니다. 여기에는 Provider probing이나 fallback 없이 admitted `api_dialect`와 complete binding에서 정확한 Connector identity 하나를 도출하는 closed registry가 포함됩니다. Flat concrete crate인 `yo-connector-openai-responses`, `yo-connector-openai-chat-completions`, `yo-connector-kimi`는 `yo-core`에 의존해야 하고 neutral replay contract와 opaque envelope를 위해서만 `yo-backend`에 의존할 수 있으며 서로 의존해서는 안 됩니다. `yo-core`는 concrete Connector에 의존하면 안 됩니다. `yo-cli`가 이들의 process-wide construction과 injection을 소유합니다. 좁은 `yo-connector-transport` helper는 독립적으로 변하는 여러 concrete Connector가 bounded HTTPS·SSE byte lifecycle mechanism을 공유하기 때문에만 허용되며, API dialect, Provider policy, semantic replay 의미, provider-private 해석의 소유자가 되면 안 됩니다.

다른 protocol, adapter, utility를 추출하려면 여전히 독립 consumer, 독립적으로 변하는 host나 service protocol 또는 process lifecycle, release boundary가 필요합니다. Directory shape, shared dependency, 한 파일의 크기를 줄이려는 의도만으로는 충분하지 않습니다.

## 이유

이 경계는 터미널과 미래 GUI frontend에 하나의 실행 엔진을 제공하면서 generic `core`라는 이름이 잡다한 utility나 Provider 구현 container가 되는 것을 막습니다. 검토된 Backend와 Model Connector ownership boundary를 허용하면서 dependency inversion을 유지합니다. Semantic core가 port를 정의하고, neutral backend foundation이 replay correlation과 bound를 소유하며, 독립적으로 변하는 adapter가 자신의 wire format을 해석하고, product composition root가 정확한 구현을 선택합니다. Shared transport helper를 byte lifecycle mechanism으로 제한하면 cancellation·cleanup code의 중복과 두 번째 숨은 semantic core를 모두 피할 수 있습니다.
