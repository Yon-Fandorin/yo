---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.backend.execution-topology
revision: sha256:3ccca0a53df133121243470c3d55f7db815065593df321c9515ca7008243d2e4
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:d36ce56c1a715d79055ec152bcf51cbb8de625fdabab3f792448d3e7d3bb8d53
---
# Korean Review Projection

## Translation

# 에이전트 백엔드 실행 토폴로지

## 계약

에이전트 백엔드는 오케스트레이션 소유권, 커넥터, 실행 위치, 전송 방식, 워크스페이스 호스트, 도구 실행 호스트라는 서로 독립적인 축으로 분류해야 합니다. 위임형 에이전트 백엔드는 Codex app-server, Grok Build ACP, Kimi Code 같은 코딩 에이전트 호스트에 연결되며, 그 호스트가 에이전트 루프와 도구 실행, 백엔드 세션을 소유합니다. Yo 관리형 백엔드는 이 책임을 yo가 유지하며 OpenAI나 Kimi 같은 서비스에 모델 커넥터로 연결합니다.

`Provider`는 위임된 코딩 에이전트 프로세스가 아니라 모델 서비스를 가리켜야 합니다. `Local`과 `Remote`는 실행 위치이고, stdio, SSH, WebSocket, HTTP, SSE는 전송 방식입니다. 어느 쪽도 별도의 의미적 백엔드 종류를 만들지 않습니다. 모든 에이전트 백엔드는 자신이 Request 진단에서 실제로 관찰할 수 있는 정확한 경계를 보고해야 하며, 커넥터가 있다면 이를 통해 보고합니다. 다른 프로세스나 서비스가 소유한 하위 요청을 볼 수 있다고 주장해서는 안 됩니다.

Generic backend lifecycle, capability, failure, evidence, replay type은 독립 `yo-backend` foundation 크레이트에 있어야 합니다. Bounded child-process JSONL, stderr 보존, request ID 발급, deferred-message mechanism을 거기서 공유할 수 있지만 host wire 해석과 Yo semantic state가 foundation에 들어가면 안 됩니다. yo-core는 `BackendAdapter`를 provider 중립 `AgentBackend` port로 특수화해야 하며 concrete backend에 의존하면 안 됩니다.

Concrete backend는 flat independent crate인 `yo-backend-managed`, `yo-backend-delegated-codex`, `yo-backend-delegated-grok`에 있어야 합니다. 각 crate는 foundation과 yo-core 특수화에 의존합니다. Process host가 admitted adapter를 선택하고 생성합니다. 현재 local delegated adapter는 Codex app-server와 Grok Build ACP입니다.

## 이유

소유권, 공급자, 실행 위치, 전송 방식을 분리하면 로컬 전용 백엔드 타입을 만들지 않고도 로컬 Codex·Grok 프로세스, 원격 에이전트 호스트, yo 자체 모델 루프에 같은 세션 의미를 적용할 수 있습니다. 독립 어댑터 크레이트는 호스트 프로토콜 변경이 의미 코어로 번지는 것을 막고, 새 호스트를 추가할 때 yo-core에 구체적인 백엔드 의존성을 더하지 않게 합니다.
