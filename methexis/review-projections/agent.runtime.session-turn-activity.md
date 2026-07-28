---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.runtime.session-turn-activity
revision: sha256:f2711f638f3fc4c25bf22c5f043a8b12510507472b838288d6db0a0dbe12d66a
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:efd6950b63bf888fe66295924c463f8385740aac4181d4ce5b39aa4cf3bcd48f
---
# Korean Review Projection

## Translation

Session은 순서가 있는 Turn들과 유지되는 에이전트 문맥을 소유해야 합니다. Turn은 사용자 요청 하나가 실행 대상으로 받아들여질 때 시작하고 completed, interrupted, failed 중 하나가 되었을 때만 끝납니다. 그 작업 안의 모델 처리, 스트리밍 응답, 도구 호출과 결과, 파일 변경, 승인 요청과 응답, 에이전트가 요청한 사용자 입력은 새 Turn이 아니라 같은 Turn의 Activity입니다.

첫 제품 host는 활성 Session 하나와 그 안의 활성 Turn 최대 하나만 허용해야 합니다. 하지만 core 계약은 모든 Session과 Turn을 명시적으로 식별하고 암묵적인 전역 current session에 의존해서는 안 됩니다. 백그라운드 프로세스처럼 Turn 이후에도 유지되어야 하는 자원은, 생성 위치를 Activity가 기록하더라도 Session이 소유합니다.

resume, fork, list, archive, rollback과 동시에 여러 Session을 불러오는 동작은 연기합니다. 초기 identity와 ownership 모델은 Session이나 Turn을 다시 정의하지 않고 이 동작들을 추가할 수 있어야 합니다.

이 계층은 도구가 섞인 에이전트 작업에 명확한 완료 경계를 제공하면서 history, branching, 여러 frontend로 확장할 수 있는 identity를 유지합니다.
