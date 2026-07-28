---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.delivery.first-coding-loop
revision: sha256:032c41cd0a0d6d0ac462c99223b6744850f6061519d8a209f1032689e194b549
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:14e28f4e6b7207538e102dc5f1437f2be40a20851b02c6c12193d3dbb1950c35
---
# Korean Review Projection

## Translation

첫 실행 가능한 에이전트 milestone은 app-server 시작과 초기화, 새 Session 하나 생성, 프롬프트 제출, 스트리밍 에이전트 텍스트, 완료된 도구 Activity 하나와 파일 변경 관찰, 승인 요청과 응답, Turn 완료 또는 중단, 명시적인 실패 보고, 자식 프로세스와 터미널 cleanup을 `yo-cli`, `yo-core`, `yo-tui`를 통해 연결해야 합니다.

milestone은 fake backend를 사용한 결정론적으로 재현 가능한 정상, 승인, 중단, 실패 경로를 제공해야 하며, 완료된 도구 event와 파일 변경 event도 포함해야 합니다. 호환되는 로컬 Codex 설치를 사용하는 환경 의존 통합 경로는 실제 도구 동작을 완료하고 일회용 workspace에서 관찰 가능한 파일 변경을 검증해야 합니다. Codex 실행 파일 누락, 초기화 또는 Session 실패, 지원하지 않거나 잘못된 protocol 입력, 예상하지 못한 자식 프로세스 종료, Turn 실패, cleanup 실패를 서로 구분할 수 있어야 합니다.

기존 Session 재개나 목록, fork, archive, rollback, queued input, WebSocket 또는 원격 transport, 여러 활성 Session, 다른 backend, GUI는 범위 밖입니다.

이는 yo가 채팅 화면 데모가 아니라 코딩 에이전트 인터페이스임을 증명하는 가장 작은 세로 Slice이며, history, distribution, multi-provider 확장은 실행 근거가 생긴 후로 남깁니다.
