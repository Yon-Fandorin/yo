---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.backend.codex-app-server
revision: sha256:2cdd82d1f384ddd65e70d99297bfdb738031fc625b317f38c15d80dc20c9435c
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:59cabd8ed9279d3f09a1f0243a8bfd36cc0097abe37e501ad1b74c79da4e07d1
---
# Korean Review Projection

## Translation

# 초기 Codex app-server 백엔드

## 계약

첫 실제 에이전트 백엔드는 로컬에 설치된 `codex app-server`의 기본 stdio JSONL 전송을 어댑트해야 합니다. 어댑터는 초기화와 프로토콜 버전 호환성 검사를 수행하고, 추가 기능을 협상할 수 있으며, 호환되지 않으면 명시적으로 실패해야 합니다. Codex Thread·Turn·Item 메시지를 yo의 Session·Turn·Activity 의미로 변환하고 Codex 전용 통신 타입은 백엔드 경계 안에 비공개로 유지해야 합니다.

`yo-cli`가 백엔드를 선택하고 연결합니다. 비공개 백엔드 모듈은 제품 프로세스 호스트와 협력하여 자식 프로세스와 결정적인 정리를 소유합니다. 같은 코어 계약에는 Codex 설치, 자격 증명, 네트워크, 비결정적인 모델 출력 없이 계약과 실패를 테스트할 수 있는 결정적 가짜 백엔드가 있어야 합니다.

WebSocket 전송, 원격 app-server 사용, 다른 위임형 에이전트 백엔드는 각각의 실행 가능한 증거가 생길 때까지 미룹니다.

## 이유

app-server가 기존 코딩 에이전트 엔진, 인증, 도구, 승인, 스트리밍 사건을 제공하므로 yo는 에이전트를 다시 구현하거나 도메인 계약을 Codex에 결합하지 않고 인터페이스를 검증할 수 있습니다.
