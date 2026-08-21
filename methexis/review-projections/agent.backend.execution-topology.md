---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.backend.execution-topology
revision: sha256:56cae3e596c3b5755eaa3a747c59c2fe27c4acdb78f59cf4b60a6b4925afa20d
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:28606fd2d601ccbabc02c1ecd59c7086bcc04141fb72cf11285e52aafb207ab1
---
# Korean Review Projection

## Translation

# 에이전트 백엔드 실행 토폴로지

## 계약

에이전트 백엔드는 오케스트레이션 소유권, 커넥터, 실행 위치, 전송 방식, 워크스페이스 호스트, 도구 실행 호스트라는 서로 독립적인 축으로 분류해야 합니다. 위임형 에이전트 백엔드는 Codex app-server, Grok Build ACP, Kimi Code 같은 코딩 에이전트 호스트에 연결되며, 그 호스트가 에이전트 루프와 도구 실행, 백엔드 세션을 소유합니다. Yo 관리형 백엔드는 이 책임을 yo가 유지하며 OpenAI나 Kimi 같은 서비스에 모델 커넥터로 연결합니다.

`Provider`는 위임된 코딩 에이전트 프로세스가 아니라 모델 서비스를 가리켜야 합니다. `Local`과 `Remote`는 실행 위치이고, stdio, SSH, WebSocket, HTTP, SSE는 전송 방식입니다. 어느 쪽도 별도의 의미적 백엔드 종류를 만들지 않습니다. 모든 에이전트 백엔드는 자신이 Request 진단에서 실제로 관찰할 수 있는 정확한 경계를 보고해야 하며, 커넥터가 있다면 이를 통해 보고합니다. 다른 프로세스나 서비스가 소유한 하위 요청을 볼 수 있다고 주장해서는 안 됩니다.

Generic backend lifecycle, capability, failure, evidence, replay type은 독립 `yo-backend` foundation 크레이트에 있어야 합니다. 그 generic replay 계약에는 정확한 durable replay에 필요한 최소한의 bounded versioned opaque provider-private envelope를 둘 수 있지만 Provider schema나 payload를 해석해서는 안 됩니다. Bounded child-process JSONL, stderr 보존, request ID 발급, deferred-message mechanism도 거기서 공유할 수 있지만 host wire 해석과 Yo semantic state가 foundation에 들어가면 안 됩니다. `yo-core`는 `BackendAdapter`를 provider 중립 `AgentBackend` port로 특수화해야 하며 concrete backend에 의존하면 안 됩니다.

Concrete backend는 flat independent crate인 `yo-backend-managed`, `yo-backend-delegated-codex`, `yo-backend-delegated-grok`에 있어야 합니다. 각 crate는 foundation과 `yo-core` 특수화에 의존합니다. Process host가 admitted adapter를 선택하고 생성합니다. 현재 local delegated adapter는 Codex app-server와 Grok Build ACP입니다.

Model Connector 경계는 Agent Backend 경계와 독립적으로 유지해야 합니다. `yo-core`는 provider 중립 Connector port와 공통 Connector semantic request, observation, failure, cancellation, complete-binding type만 소유해야 하며, 여기에는 admitted `api_dialect`와 complete binding에서 정확한 Connector identity를 도출하는 closed registry가 포함됩니다. 정확한 HTTP request 구성, dialect stream decoding, endpoint policy, retry grammar, provider-private payload 해석은 `yo-core`, `yo-backend`, `yo-backend-managed`에 들어가면 안 됩니다.

Concrete Model Connector는 `crates/connectors/` 아래의 flat independent crate인 `yo-connector-openai-responses`, `yo-connector-openai-chat-completions`, `yo-connector-kimi`로 유지해야 합니다. 각 crate는 `yo-core`에 의존해야 하고 connector-neutral replay contract와 opaque provider-private envelope를 위해서만 `yo-backend`에 의존할 수 있으며, 자신에게 admitted된 정확한 Connector identity와 dialect를 구현해야 하고 다른 concrete Connector에 의존해서는 안 됩니다. `yo-core`, `yo-backend`, `yo-backend-managed`는 concrete Connector에 의존하면 안 됩니다. Kimi request·response grammar, private-assistant schema decoding과 codec, lossless validation, connector-neutral visible replay projection 추출, 정확한 encoded-size 계산은 오직 `yo-connector-kimi`에 속합니다. 이 crate는 검증된 projection을 bounded opaque provider-private envelope와 함께 반환해야 합니다. `yo-backend`는 envelope를 보존하고 bound할 수 있지만 Kimi field를 해석하면 안 되며, `yo-backend-managed`는 envelope가 선언한 schema identity, binding epoch, bound만 검증하고 Connector가 제공한 projection을 semantic replay와 비교할 수 있습니다.

`crates/connectors/transport` 아래의 flat internal `yo-connector-transport` crate는 적어도 두 concrete Connector가 공유할 때만 bounded HTTPS·SSE byte transport, framing, cancellation, cleanup, delivery mechanism을 위해 사용할 수 있습니다. API dialect, Provider나 Model policy, complete binding, semantic replay 의미, retry 결정, provider-private payload 해석을 소유하면 안 됩니다. Concrete Connector는 자신의 request grammar, response terminal, retry admission, semantic projection의 유일한 소유자로 남아야 합니다.

`yo-core`는 closed `api_dialect` registry를 통해 Provider probing이나 fallback 없이 정확한 Connector identity 하나를 도출해야 합니다. Process-wide composition owner인 `yo-cli`는 이미 도출된 정확한 Connector identity와 dialect를 concrete factory 하나에 매핑하여 `yo-backend-managed`와 model-service verification 경로에 주입해야 합니다. 이 composition은 Provider를 probe하거나 Model 이름에서 dialect를 추론하거나 다른 Connector로 fallback하거나 managed loop가 Provider에 따라 분기하게 해서는 안 됩니다. 이 분리는 migration 없이 기존 Journal byte와 ordering, binding epoch, replay profile, visibility exclusion, plaintext-retention consent, request behavior, terminal behavior를 보존해야 합니다.

## 이유

소유권, 공급자, 실행 위치, 전송 방식을 분리하면 로컬 전용 백엔드 타입을 만들지 않고도 로컬 Codex·Grok 프로세스, 원격 에이전트 호스트, yo 자체 모델 루프에 같은 세션 의미를 적용할 수 있습니다. 독립 어댑터 크레이트는 호스트 프로토콜 변경이 의미 코어로 번지는 것을 막고, 새 호스트를 추가할 때 `yo-core`에 구체적인 백엔드 의존성을 더하지 않게 합니다.

허용된 세 model dialect는 이미 서로 독립적으로 변하며 Kimi는 private replay와 provider-specific request 규칙도 소유합니다. Flat Connector crate는 그 변경을 `yo-core`와 managed loop 밖에 두고, 기존 neutral replay foundation에는 correlation, bound, opaque durable payload만 남깁니다. 좁은 transport helper 하나는 두 번째 semantic owner가 되지 않으면서 byte lifecycle mechanism의 복제를 피합니다. Process root에서의 주입은 정확한 binding 선택을 보존하며, 터미널이나 미래 GUI frontend가 모든 concrete Provider 구현을 가져오지 않고 같은 semantic engine을 재사용하게 합니다.
