---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.observability.view-projections
revision: sha256:322bcdbf7a3066270d3a62f22968d22a6b67f6c1cfb4d633e779c8f6f77d95fa
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:89c80347c12f50dbb0f1413f7ec8fd51707dc91ceb47806b5ed0d72f2b8ebb2a
---
# Korean Review Projection

## Translation

# Chat, Transcript, Request 화면

## 계약

Chat, Transcript, Request에 표시되는 이력은 모두 같은 읽기 전용 세션 저널 투영에서 파생되어야 하며, 각 화면이 별도의 원본이 되어서는 안 됩니다. Chat은 사용자가 입력할 수 있는 기본 상호작용 화면이어야 합니다. 기존에 확립된 코딩 에이전트 상호작용을 따라 간결한 의도, 의미 있는 도구·파일 활동, 테스트, 승인, 오류, 결과를 보여주고 반복 탐색과 긴 출력은 접어야 합니다.

Transcript는 Chat을 포함하는 투명한 시간순 화면이며, 상세한 의미 사건과 활동 생명주기, 문맥, 실패, 관찰·영속 공백을 추가로 보여줘야 합니다. Request는 현재 Chat 또는 Transcript에서 보고 있는 문맥에 연결된 전체 화면 읽기 전용 투영이며, 주된 형태가 요청 목록 탐색기가 되어서는 안 됩니다. 연결된 백엔드 통신, 수정본, 시도, 결과, 민감정보 제거, 정확한 관찰 경계를 보여줘야 합니다. 직접 연결된 요청이 없는 문맥에서는 가까운 다른 요청을 대신 선택하지 말고 요청이 없다고 밝혀야 합니다. 연결된 화면 사이를 오간 뒤에는 각 화면의 커서와 스크롤 위치를 복원해야 합니다.

## 이유

하나의 재생 원본을 사용하면 기본 대화에 모든 세부 정보를 밀어 넣지 않고도 TUI와 미래 GUI에서 간결한 작업 흐름, 투명한 시간순 이력, 통신 수준 진단을 서로 일치시킬 수 있습니다.
