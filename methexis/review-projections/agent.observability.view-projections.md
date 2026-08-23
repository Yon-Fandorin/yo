---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.observability.view-projections
revision: sha256:34b55065976c0a565d78b0fff37c9ade6afd67d30d649e5ea5fea59e512a6a9f
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:ba1c8dbfbd7990e42e61d0f5e3d5056b7418baead555f77bd637224fb4065d9c
---
# Korean Review Projection

## Translation

# Chat, Transcript, Request, Usage 화면

## 계약

Chat과 Transcript에 표시되는 이력은 읽기 전용 의미적 세션 저널에서 파생되어야 합니다. Request는 저널의 제한된 연결·가용성 레코드와 같은 세션 생명주기 아래의 선택적 Request Audit 상세를 결합하는 읽기 전용 진단 투영이어야 합니다. 투영이나 상세가 별도 원본이 되어서는 안 됩니다.

Chat은 사용자가 입력할 수 있는 기본 상호작용 화면이어야 합니다. 기존에 확립된 코딩 에이전트 상호작용을 따라 간결한 의도, 의미 있는 도구·파일 활동, 테스트, 승인, 오류, 결과를 보여주고 반복 탐색과 긴 출력은 접어야 합니다. Transcript는 Chat을 포함하는 투명한 시간순 화면이며, 상세한 의미 사건과 활동 생명주기, 문맥, 실패, 관찰·영속 공백을 추가로 보여줘야 합니다.

Request는 해당 세션 전체에 속한 저널의 모든 제한된 연결·가용성 레코드를 시간순으로 보여주는 전체 화면 읽기 전용 진단 흐름이어야 하며, 요청 목록 탐색기가 되어서는 안 됩니다. 관찰 가능한 백엔드 통신, 수정본, 시도, 결과, 민감정보 제거, 정확한 관찰 경계와 상세를 사용할 수 없는 타입화된 이유를 보여줘야 합니다. 대화형 화면은 이 흐름 안에서 현재 Chat 또는 Transcript 문맥을 강조할 수 있습니다. 강조된 문맥에 직접 연결된 요청이 없으면 가까운 다른 요청을 대신 선택하지 말고 요청이 없다고 밝혀야 합니다. 연결된 화면 사이를 오간 뒤에는 각 화면의 커서와 스크롤 위치를 복원해야 합니다. 미래 원격 reader의 on-demand 상세 조회는 실제 원격 소비자가 그 계약을 정의한 뒤에만 추가할 수 있으며, 이 결정만으로 원격 Request Audit 인터페이스가 생기지는 않습니다.

Session Usage는 완료된 ModelWork Activity의 usage receipt만 대상으로 하는 읽기 전용 투영이어야 합니다. 보관 세션의 CLI Usage와 라이브 F4 Usage는 하나의 타입화된 공용 투영을 소비하여 동일한 의미를 가져야 하며, 어느 프런트엔드도 receipt를 독립적으로 해석하거나 집계해서는 안 됩니다. 이 투영은 receipt 시간순을 보존해야 합니다. 각 토큰 집계는 완전, 부분적, 사용할 수 없음 중 하나여야 합니다. 부분적이거나 사용할 수 없는 집계에는 포함/전체 receipt 커버리지(x/y)를 표시하여 누락된 값을 완전한 값처럼 보여서는 안 됩니다. Cache-read 비율에는 cache-read 토큰 데이터가 명시되고 입력 토큰 분모를 알 수 있는 receipt만 포함해야 합니다. 그 토큰 분모에는 그러한 적격 receipt의 알려진 입력 토큰만 포함해야 하며, 적격/전체 receipt 커버리지를 표시해야 합니다. 인식된 완료 receipt가 없는 Session도 빈 투영으로 성공해야 합니다. 인식된 receipt 스키마에서는 보고된 0, 필드 없음, 미지원 상태를 서로 구분해야 하며, 잘못된 데이터가 있으면 전체 투영을 fail-closed 해야 합니다. Codex 집계에는 turn별 usage만 사용하고 누적 thread_total은 제외해야 합니다. Usage는 비용, 과금, cache hit, 비캐시 토큰, 누락된 귀속, 프로바이더 간 cache-write 동등성을 추론해서는 안 되며 원시 요청·응답, 자격 증명, 비공개 추론 내용을 노출해서는 안 됩니다.

## 이유

하나의 의미적 재생 원본은 간결한 작업 흐름과 투명한 시간순 이력을 일치시킵니다. 선택적으로 연결된 상세는 의미 저널에 모든 wire 데이터를 넣지 않고도 TUI와 미래 GUI에서 통신 수준 진단을 가능하게 합니다. 하나의 타입화된 공용 Usage 투영도 receipt 해석을 중복하지 않으면서 보관 화면과 라이브 화면의 의미를 일치시킵니다.
