---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.backend.codex-app-server
revision: sha256:dc2f48f5d235ed6234387abf8558022548437f6a42d81372a6a659a21bd3fcb2
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:2cbf04bc4758fc3299d5fcca8fee0cc6fa413c81dbc67b56a25af460881a5d5d
---
# Korean Review Projection

## Translation

# 초기 Codex app-server 백엔드

## 계약

첫 실제 에이전트 백엔드는 로컬에 설치된 `codex app-server`의 기본 stdio JSONL 전송을 어댑트해야 합니다. 어댑터는 초기화와 프로토콜 버전 호환성 검사를 수행하고, 추가 기능을 협상할 수 있으며, 호환되지 않으면 명시적으로 실패해야 합니다. Codex Thread·Turn·Item 메시지를 yo의 Session·Turn·Activity 의미로 변환하고 Codex 전용 통신 타입은 백엔드 경계 안에 비공개로 유지해야 합니다.

`yo-cli`가 백엔드를 선택하고 연결합니다. 비공개 백엔드 모듈은 제품 프로세스 호스트와 협력하여 자식 프로세스와 결정적인 정리를 소유합니다. 같은 코어 계약에는 Codex 설치, 자격 증명, 네트워크, 비결정적인 모델 출력 없이 계약과 실패를 테스트할 수 있는 결정적 가짜 백엔드가 있어야 합니다.

WebSocket 전송, 원격 app-server 사용, 다른 위임형 에이전트 백엔드는 각각의 실행 가능한 증거가 생길 때까지 미룹니다.


Codex binding은 backend_managed_state continuation을 명시합니다. Yo는 durable transcript, semantic event, correlation record, versioned Codex Thread locator를 소유하고 Codex는 model-visible conversation state를 소유합니다. Resume은 locator로 reconnect한 뒤 versioned identity schema에 따라 반환된 Thread identity를 검증합니다. Completed resumable Codex Turn은 payload-free outcome과 Continuation Anchor를 기록하지만 model_replay_delta와 replay_delta_sequence는 기록하지 않습니다. Provider Response 또는 item identity는 correlation evidence일 수 있으나 Yo exact replay로 표현하지 않습니다.

## 이유

app-server가 기존 코딩 에이전트 엔진, 인증, 도구, 승인, 스트리밍 사건을 제공하므로 yo는 에이전트를 다시 구현하거나 도메인 계약을 Codex에 결합하지 않고 인터페이스를 검증할 수 있습니다.
