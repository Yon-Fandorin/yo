---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.core.frontend-independent-boundary
revision: sha256:3a218c1ebcf9285db71752242e0348ed69b78ae3592c6503dbb874e405507038
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:1a10381baa0ef901601c5712eae1c61f5bf3f03377ce0ab98f3bec63d398e67c
---
# Korean Review Projection

## Translation

공유 에이전트 엔진의 이름은 `yo-core`여야 하며, 프런트엔드와 무관한 에이전트 실행 의미만 소유해야 합니다. `yo-tui`는 UI 동작을, `yo-cli`는 제품 진입점과 프로세스 전역 생명주기 정책 및 최상위 연결을 소유합니다. 미래 GUI는 `yo-tui`에 의존하지 않고 `yo-core`를 재사용해야 합니다.

여러 곳에서 사용된다는 이유만으로 코드를 `yo-core`에 넣어서는 안 되며, 에이전트 실행 의미를 표현할 때만 이 크레이트에 속합니다. 초기 모놀리스는 의미 경계가 안정화되는 동안 session, command, event, configuration, backend 관심사를 함께 유지했습니다. 이후의 크레이트 분리는 공유된다는 이유만으로 코드를 추출하는 것이 아니라 독립적으로 변하는 구체적인 아키텍처 경계를 식별해야 합니다.

승인된 backend 분리는 provider 중립 `yo-backend` foundation, `yo-core`의 `AgentBackend` 의미 특수화, 그리고 flat concrete crate인 `yo-backend-managed`, `yo-backend-delegated-codex`, `yo-backend-delegated-grok`으로 구성됩니다. Concrete backend는 foundation과 `yo-core`에 의존할 수 있지만 `yo-core`와 foundation은 concrete backend에 의존해서는 안 됩니다. 그 밖의 protocol, adapter, utility 추출에는 여전히 독립 소비자, 독립적으로 변하는 host protocol 또는 process lifecycle, 또는 release boundary가 필요합니다.

이 경계는 터미널과 미래 GUI가 하나의 실행 엔진을 사용하게 하면서도, 일반적인 `core`라는 이름이 잡다한 유틸리티 저장소가 되는 것을 막습니다. 또한 검토된 backend 소유 경계를 허용하면서 모든 공유 mechanism이 근거 없이 별도 crate가 되는 것을 막습니다.
