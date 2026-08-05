---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.observability.view-projections
revision: sha256:d6dfe4c07c30988c606a93b9d0f9076f46bbe9ce63931ffb7917a1d020351c29
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:3d4d7dc8c08d64da6efa0eae003862fd0c07581c8e0674721e5ac3c0bd2a08eb
---
# Korean Review Projection

## Translation

# Chat, Transcript, Request 화면

## 계약

Chat과 Transcript에 표시되는 이력은 읽기 전용 의미적 세션 저널에서 파생되어야 합니다. Request는 저널의 제한된 연결·가용성 레코드와 같은 세션 생명주기 아래의 선택적 Request Audit 상세를 결합하는 읽기 전용 진단 투영이어야 합니다. 투영이나 상세가 별도 원본이 되어서는 안 됩니다.

Chat은 사용자가 입력할 수 있는 기본 상호작용 화면이어야 합니다. 기존에 확립된 코딩 에이전트 상호작용을 따라 간결한 의도, 의미 있는 도구·파일 활동, 테스트, 승인, 오류, 결과를 보여주고 반복 탐색과 긴 출력은 접어야 합니다. Transcript는 Chat을 포함하는 투명한 시간순 화면이며, 상세한 의미 사건과 활동 생명주기, 문맥, 실패, 관찰·영속 공백을 추가로 보여줘야 합니다.

Request는 해당 세션 전체에 속한 저널의 모든 제한된 연결·가용성 레코드를 시간순으로 보여주는 전체 화면 읽기 전용 진단 흐름이어야 하며, 요청 목록 탐색기가 되어서는 안 됩니다. 관찰 가능한 백엔드 통신, 수정본, 시도, 결과, 민감정보 제거, 정확한 관찰 경계와 상세를 사용할 수 없는 타입화된 이유를 보여줘야 합니다. 대화형 화면은 이 흐름 안에서 현재 Chat 또는 Transcript 문맥을 강조할 수 있습니다. 강조된 문맥에 직접 연결된 요청이 없으면 가까운 다른 요청을 대신 선택하지 말고 요청이 없다고 밝혀야 합니다. 연결된 화면 사이를 오간 뒤에는 각 화면의 커서와 스크롤 위치를 복원해야 합니다. 미래 원격 reader의 on-demand 상세 조회는 실제 원격 소비자가 그 계약을 정의한 뒤에만 추가할 수 있으며, 이 결정만으로 원격 Request Audit 인터페이스가 생기지는 않습니다.

## 이유

하나의 의미적 재생 원본은 간결한 작업 흐름과 투명한 시간순 이력을 일치시킵니다. 선택적으로 연결된 상세는 의미 저널에 모든 wire 데이터를 넣지 않고도 TUI와 미래 GUI에서 통신 수준 진단을 가능하게 합니다.
