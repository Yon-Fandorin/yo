---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.runtime.active-turn-input
revision: sha256:39aa3a6547d3c30c50ae361dd884d9ce66b20262d2b62bdd3936f216f5a5735e
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:9ae1d548fade403be6b983097890c7fd30167cc1c1abe05683881f1f7d9c077a
---
# Korean Review Projection

## Translation

Turn이 실행 중일 때 일반 프롬프트를 제출하면 초기 TUI에서는 식별된 현재 Turn을 대상으로 하는 steer 요청으로 해석하여 제출해야 합니다. 대기 중인 승인이나 에이전트가 요청한 입력에 대한 응답은 Activity 응답이며, steer나 새 Turn이 아닙니다.

이후 Turn을 위해 입력을 queue하는 기능은 별도의 연기된 동작입니다. 선택한 backend가 steer를 지원하지 않으면 `yo-core`는 명시적인 unsupported 결과를 반환해야 하며, 해당 입력을 조용히 queue 작업으로 바꾸어서는 안 됩니다.

steer와 queue의 의미를 명시하면 숨겨진 backend capability에 따라 사용자 입력의 시간적 의도가 달라지는 일을 막으면서, 실행 중인 에이전트 작업을 즉시 교정할 수 있습니다.
