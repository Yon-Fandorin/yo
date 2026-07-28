---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.core.frontend-independent-boundary
revision: sha256:48bdc83d2ff51aec31703a19c69bfe7cf8767eff6f55c488f531ff247b5dbdce
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:c21615aba7c731e382a9cde7c257bdfe13fc2f292770233c4279911449df4a6b
---
# Korean Review Projection

## Translation

공유 에이전트 엔진의 이름은 `yo-core`여야 하며, 프런트엔드와 무관한 에이전트 실행 의미만 소유해야 합니다. `yo-tui`는 UI 동작을, `yo-cli`는 제품 진입점과 프로세스 전역 생명주기 정책 및 최상위 연결을 소유합니다. 미래 GUI는 `yo-tui`에 의존하지 않고 `yo-core`를 재사용해야 합니다.

여러 곳에서 사용된다는 이유만으로 코드를 `yo-core`에 넣어서는 안 됩니다. 에이전트 실행 의미를 표현할 때만 이 크레이트에 속합니다. 초기 구현은 session, command, event, configuration, backend 관심사를 하나의 크레이트 내부 모듈로 유지해야 합니다. 별도 protocol 또는 adapter 크레이트는 독립 소비자나 릴리스 경계가 생겼을 때만 추가합니다.

이 경계는 터미널과 미래 GUI가 하나의 실행 엔진을 사용하게 하면서도, 일반적인 `core`라는 이름이 잡다한 유틸리티 저장소가 되거나 근거 없이 크레이트가 분리되는 것을 막습니다.
