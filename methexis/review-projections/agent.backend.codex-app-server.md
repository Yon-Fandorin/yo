---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.backend.codex-app-server
revision: sha256:901b6948d8834b2dc771592a0152dc8f05da8f6953e98ae9c94c3d13b208ac3e
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:8d25aa6b932f5ef3857e65a6f1f54157afd0b5788e3070a78ace86475b48b00d
---
# Korean Review Projection

## Translation

첫 실제 에이전트 backend는 로컬에 설치된 `codex app-server`를 기본 stdio JSONL transport로 연결해야 합니다. adapter는 초기화와 protocol version 호환성 검사를 수행하고, 추가 capability를 협상할 수 있으며, 비호환일 때 명시적으로 실패해야 합니다. Codex의 Thread/Turn/Item 메시지를 yo의 Session/Turn/Activity 의미로 변환하고, 모든 Codex 전용 wire type을 backend 경계 안에 비공개로 유지해야 합니다.

`yo-cli`가 backend를 선택하고 연결합니다. 비공개 backend 모듈은 제품 process host와 협력하여 자식 프로세스와 결정론적으로 재현 가능한 cleanup을 소유합니다. 같은 core 계약에는 Codex 설치, 인증 정보, 네트워크 접근, 비결정적인 모델 출력 없이 contract와 failure test를 실행할 수 있는 결정론적으로 재현 가능한 fake backend가 있어야 합니다.

WebSocket transport, 원격 app-server 사용, 다른 provider backend는 각각의 실행 가능한 근거가 생길 때까지 연기합니다.

app-server가 기존 코딩 에이전트 엔진, 인증, 도구, 승인, 스트리밍 event를 제공하므로 yo는 에이전트를 다시 구현하거나 도메인 계약을 Codex에 결합하지 않고 인터페이스를 검증할 수 있습니다.
