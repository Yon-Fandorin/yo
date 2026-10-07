# 화면 예시 목차와 검수 범위

**66개 대표 장면 → 64개 상위 요소 + 10개 콘텐츠 하위 요소**의 추적표다.
매핑은 검토할 화면 witness를 뜻한다. 모든 상태/전이/키 입력을 실행해 검증했다는 뜻이 아니다.
동적 계약은 아래 별도 실행 항목을 통과해야 한다. source pin과 현재/제안 차이는 [출처](sources.md)에 있다.

## 장면과 이후 동작

| 장면 | 상태 | 80열 일반 텍스트 | 이후 동작의 설계 |
|---|---|---|---|
| 01-start · 시작 | 연결된 모델 없음 | [보기](plain/01-start.80x30.txt) | 연결 완료 후 빈 일반 초안으로 돌아온다. 연결 취소는 설정을 변경하지 않는다. |
| 02-connect · 모델 연결 | 연결 대기 · 00:24 | [보기](plain/02-connect.80x30.txt) | 성공 시 모델 목록. 만료/거절 시 이유와 재연결 행동. 대화 초안은 보존한다. |
| 03-model · 모델 선택 | 검색: code · 2개 | [보기](plain/03-model.80x30.txt) | 수락 후 예약 상태를 표시하고 실제 전환 성공 때 현재 모델을 바꾼다. 실패 시 현재 모델/첨부를 보존한다. |
| 04-model-failed · 모델 전환 | 실패 · 현재 모델 유지 | [보기](plain/04-model-failed.80x30.txt) | 모델 재선택 또는 첨부 제거를 별도 행동으로 수행한다. 자동 재전송하지 않는다. |
| 05-settings · 화면 설정 | 적용 범위: 현재 세션 | [보기](plain/05-settings.80x30.txt) | 현재 보기 변경과 기본값 저장을 분리한다. 저장 실패 시 현재 값과 미저장 상태를 함께 표시한다. |
| 06-settings-failed · 화면 설정 | 기본값 저장 실패 | [보기](plain/06-settings-failed.80x30.txt) | 재시도 전 현재 설정을 잃지 않는다. 실패를 성공 알림으로 바꾸지 않는다. |
| 07-chat · Chat | 대기 · 입력 가능 | [보기](plain/07-chat.80x30.txt) | Enter는 일반 입력 수락 요청이다. 수락 확인 전에 초안을 완료된 메시지로 취급하지 않는다. |
| 08-working-queue · Chat | 실행 중 · 00:18 | [보기](plain/08-working-queue.80x30.txt) | 중단은 현재 턴에 요청한다. 대기 메시지를 자동 삭제하지 않는다. 큐 편집은 항목 ID와 첨부를 보존한다. |
| 09-queue-paused · 대기 메시지 | 일시 정지 · 2개 | [보기](plain/09-queue-paused.80x30.txt) | 선택 항목 전용 편집으로 이동한다. 취소 시 원래 일반 초안으로 돌아온다. 삭제/재개는 별도 명시 행동. |
| 10-queue-edit · 메시지 편집 | 대기열 일시 정지 | [보기](plain/10-queue-edit.80x30.txt) | 같은 #q2 내용만 교체하고 일시 정지를 유지한다. 취소 시 편집 전 메시지로 복원한다. |
| 11-references · 참조 선택 | 검색: @parser | [보기](plain/11-references.80x30.txt) | 참조 snapshot을 초안에 삽입한다. 같은 이름의 skill은 source/scope로 구별한다. template 결과를 자동 명령 실행하지 않는다. |
| 12-history · 입력 기록 | 검색: 경계값 | [보기](plain/12-history.80x30.txt) | 기록은 편집 가능한 입력으로만 불러온다. 자동 제출하지 않는다. |
| 13-find · 대화 찾기 | 범위: 대화 + 도구 | [보기](plain/13-find.80x30.txt) | 결과의 항목과 source 위치로 이동한다. 늦은 검색 결과가 새 질의를 덮지 않는다. |
| 14-approval · 승인 요청 | Proposed · 미실행 | [보기](plain/14-approval.80x30.txt) | 선택한 요청 ID/공개 frame이 유효할 때만 응답한다. 표시한 실행 범위 이상으로 권한을 확대하지 않는다. Ctrl+V 상세 후 Esc로 돌아온다. 방향키로 선택을 바꾸면 Enter 의미도 함께 바뀐다. |
| 15-stop-approval · 승인 요청 | Proposed · 미실행 | [보기](plain/15-stop-approval.80x30.txt) | Esc는 Decline 응답을 만들지 않고 턴 중단 요청을 보낸다. 지연된 응답은 폐기한다. |
| 16-approval-detail · 승인 상세 | Proposed · 미실행 | [보기](plain/16-approval-detail.80x30.txt) | 읽던 상세 위치를 보관하고 동일한 요청의 선택으로 돌아간다. 만료된 요청이면 수락을 비활성화한다. |
| 17-question · 질문 1/2 | 답변 입력 · 요청 #r8 | [보기](plain/17-question.80x30.txt) | 답변 저장 후 질문 2/2로 이동한다. 모든 답변 검토 후 전체 제출. 일반 초안은 질문 종료 시 복원한다. |
| 18-question-choice · 질문 2/2 | 메모 미지원 | [보기](plain/18-question-choice.80x30.txt) | Tab은 선택 이동만 한다는 제안. 실제 구현 전 request capability와 dispatch를 함께 변경해야 한다. |
| 19-draft-restored · Chat | 질문 완료 · 입력 복원 | [보기](plain/19-draft-restored.80x30.txt) | 복원된 일반 초안은 사용자가 보낼 때만 새 입력이 된다. |
| 20-secret · 비밀 입력 | 입력됨 · 길이 비공개 | [보기](plain/20-secret.80x30.txt) | 명시적 전체 지우기 뒤 Not entered 상태로 전환한다. 이 예시 파일에는 실제 토큰/가짜 토큰 바이트도 없다. Ctrl+U는 기존 kill-line과 다른 전체 지우기로 바꾸는 제안이다. |
| 21-secret-cleared · 비밀 입력 | 미입력 · 전체 지움 | [보기](plain/21-secret-cleared.80x30.txt) | 입력이 없을 때 제출을 막는다. 새 값을 입력하면 입력됨 상태로 전환한다. |
| 22-secrets · 비밀 관리 | 값은 표시하지 않음 | [보기](plain/22-secrets.80x30.txt) | 관리 화면은 metadata만 보여준다. 삭제 대상과 영향을 확인한 뒤 지정된 저장 값만 제거한다. |
| 23-changes-proposed · Changes | Proposed · 미적용 | [보기](plain/23-changes-proposed.80x30.txt) | 복귀 시 원래 승인 요청과 선택을 유지한다. 여기에서 Recorded로 바꾸지 않는다. |
| 24-changes-recorded · Changes | Recorded · 실행 기록 | [보기](plain/24-changes-recorded.80x30.txt) | 실행 시점의 기록을 유지한다. 현재 파일 상태와 일치한다고 추정하지 않는다. |
| 25-changes-current · 작업 폴더 | Current · 지금 조회 | [보기](plain/25-changes-current.80x30.txt) | 새 조회에서만 갱신한다. Proposed/Recorded 증거를 덮어쓰지 않는다. |
| 26-output · Output | 실패 · exit 1 | [보기](plain/26-output.80x30.txt) | 선택한 call의 보관된 원문 위치를 연다. /output 최신 항목과 선택 항목 접근을 구별하는 제안. |
| 27-source · 원문 보기 | 보관된 원문 · 수정 없음 | [보기](plain/27-source.80x30.txt) | 원문 바이트를 유지해 복사를 요청한다. 화면의 줄바꿈을 원문 줄바꿈으로 취급하지 않는다. |
| 28-results · 작업 결과 | 완료 · 예시 증거 | [보기](plain/28-results.80x30.txt) | 각 요약은 실제 call/변경 기록 상세로 연결해야 한다. 이 fixture의 성공 수치는 실행 결과가 아닌 합성 값이다. |
| 29-sessions · 이전 작업 | 이 프로젝트 · 검색: parser | [보기](plain/29-sessions.80x30.txt) | 선택 ID의 재개가 성공한 후에만 현재 세션을 전환한다. 실패/취소 시 검색과 일반 초안을 복원한다. |
| 30-session-error · 이전 작업 | 재개 실패 · 현재 세션 유지 | [보기](plain/30-session-error.80x30.txt) | 오류 뒤에도 원래 picker의 선택과 검색을 유지한다. |
| 31-fork · 분기 시작점 | 상속 범위 확인 | [보기](plain/31-fork.80x30.txt) | 선택한 anchor 기준으로 새 세션을 만든다. 불가능한 지점은 이유를 보이고 수락하지 않는다. |
| 32-diagnostics · 진단 | Transcript / Request | [보기](plain/32-diagnostics.80x30.txt) | 상세 진단과 사용자 결과 탐색을 분리한다. 알 수 없는 사용량을 0으로 표시하지 않는다. |
| 33-rescue · 기록 저장 실패 | History not saved | [보기](plain/33-rescue.80x30.txt) | 복구 대상/제외 범위를 먼저 표시한다. 실제 파일 쓰기 확인 후에만 저장 완료를 알린다. |
| 34-export-error · 내보내기 | 파일 저장 실패 | [보기](plain/34-export-error.80x30.txt) | 경로 변경 뒤 저장 성공/실패를 확인한다. 복사 요청 전송을 복사 성공으로 표현하지 않는다. |
| 35-storage · 기록 보관 | 예시: 820 MiB 사용 | [보기](plain/35-storage.80x30.txt) | 보관 성공 확인 후 원본 처리 정책을 적용한다. 삭제는 명시한 대상/범위 확인 후 수행한다. |
| 36-reconnect · 연결 복구 | 전달 결과 불명 | [보기](plain/36-reconnect.80x30.txt) | 관찰 중단은 서버 작업 취소가 아니다. 재연결/실패/receipt 결과를 분리하고 확인 전 재전송을 막는다. |
| 37-help · 현재 화면 도움말 | Chat · 기본 키 프로토콜 | [보기](plain/37-help.80x30.txt) | 실제 capability와 활성 상태에서 키 안내를 생성해야 한다. 이 fixture의 모든 키가 현재 Yo에 연결된 것은 아니다. |
| 38-compaction · 대화 정리 | 실행 중 · 수동 요청 | [보기](plain/38-compaction.80x30.txt) | 완료/실패/취소를 구분한다. 취소 후 메모리/초안을 버리지 않는다. 정확한 지원 여부는 구현 때 계약으로 확인한다. |
| 39-compaction-failed · 대화 정리 | 실패 · 기존 기록 유지 | [보기](plain/39-compaction-failed.80x30.txt) | Enter로 명시적 재요청. Esc는 기존 대화로 복귀. 자동 재시도/완료는 60/61번 별도 장면. |
| 40-plan · 작업 계획 | Proposed plan · 미확정 | [보기](plain/40-plan.80x30.txt) | 초안 계획과 확정된 계획을 구분한다. 최종 문서가 도착하면 같은 source identity로 갱신한다. |
| 41-tasks · 병렬 작업 | 2 실행 · 1 승인 대기 | [보기](plain/41-tasks.80x30.txt) | 상태는 host 결과로 갱신한다. 화면만으로 병렬 승인/실행이 구현됐다고 간주하지 않는다. |
| 42-memory · 프로젝트 메모 | 수동 관리 · 제안 | [보기](plain/42-memory.80x30.txt) | 명시한 파일/범위만 변경한다. 숨은 자동 학습 기능을 전제하지 않는다. |
| 43-mcp · 도구 연결 | 호스트: native | [보기](plain/43-mcp.80x30.txt) | 실제 서버 호환 검증 전 사용 가능 표시를 하지 않는다. 설정 실패는 기존 세션을 유지한다. |
| 44-terminal · 터미널 표시 | SSH / tmux · 예시 환경 | [보기](plain/44-terminal.80x30.txt) | 실제 probe/설정 값으로만 능력을 표시한다. 이 fixture는 SSH/tmux/Unicode17 동작 검증이 아니다. |
| 45-documents · 문서 | 원문 유지 · 좁은 폭 재배치 | [보기](plain/45-documents.80x30.txt) | 표의 열 값/순서를 보존한다. 렌더러 실패 시 원문을 열 수 있어야 한다. |
| 46-resources · 도구 자료 | 일부 결과 · 원문 보존 | [보기](plain/46-resources.80x30.txt) | 각 자료 유형의 원래 필드와 source를 보존한다. unknown을 0이나 전체 결과로 바꾸지 않는다. |
| 47-media · 첨부와 미디어 | 텍스트 대체 표시 | [보기](plain/47-media.80x30.txt) | 표시 여부와 전송 payload를 별개로 관리한다. 지원하지 않는 미디어를 자동 실행하지 않는다. |
| 48-interview · 질문 작업 사본 | 기존 사본 관리 | [보기](plain/48-interview.80x30.txt) | 기존 사본의 lifecycle만 조작한다. 닫기/폐기 뒤 일반 초안 소유권을 복원한다. |
| 49-preview · 개발용 Preview | 제품 세션과 분리 | [보기](plain/49-preview.80x30.txt) | preview 상태를 제품 세션에 쓰지 않고 진입 전 상태를 복원한다. |
| 50-editor-failed · Chat | 외부 편집 결과 미적용 | [보기](plain/50-editor-failed.80x30.txt) | 외부 편집 실패가 기존 초안/첨부를 덮지 않는다. raw mode/cursor 복원은 실제 PTY 테스트 필요. |
| 51-picker-stale · 모델 선택 | 목록 갱신 중 | [보기](plain/51-picker-stale.80x30.txt) | 공개한 선택과 현재 항목 ID가 일치한 뒤만 수락한다. 갱신 실패는 원인을 표시한다. |
| 52-scrollback · Chat | 이전 내용 읽는 중 | [보기](plain/52-scrollback.80x30.txt) | source anchor를 보존하는 설계 제안. fixture 폭 변환은 실제 스크롤/resize 실행 검증이 아니다. |
| 53-reconnect-restored · 연결 복구 | 복구됨 · 중복 전송 없음 | [보기](plain/53-reconnect-restored.80x30.txt) | 조회 결과가 없음/불명이라면 계속 불명 상태를 유지한다. 이 성공 예시는 실행 증거가 아니다. |
| 54-empty-list · 이전 작업 | 이 프로젝트 · 0개 | [보기](plain/54-empty-list.80x30.txt) | 범위를 넓힌 뒤에도 없는 경우 빈 이유를 유지한다. 로딩과 빈 결과는 별도 상태. |
| 55-tiny · 승인 | Proposed | [보기](plain/55-tiny.80x30.txt) | 24x8 예시: 크기 부족이면 수락을 잠그고 명시한 중단 행동만 유지한다. |
| 56-palette · 명령 찾기 | 입력: /mo · 1개 | [보기](plain/56-palette.80x30.txt) | 선택한 명령을 활성 화면에 맞게 dispatch한다. 목록 갱신 중 이전 행 번호로 실행하지 않는다. |
| 57-actions · 현재 화면 행동 | 질문 #r8 · 제안 메뉴 | [보기](plain/57-actions.80x30.txt) | 선택된 행동 ID를 실행하고 이전 focus로 복귀한다. Ctrl+K는 제안 binding이며 기존 kill-line과 충돌 조정이 필요하다. |
| 58-editing · 입력 편집 | 선택 영역 · 초안만 수정 | [보기](plain/58-editing.80x30.txt) | 이 도식은 실제 selection/cursor 시뮬레이션이 아니다. caret와 selection 렌더링 및 IME는 실제 terminal test 대상. |
| 59-changes-reported · Changes | Reported · 보고 자료 | [보기](plain/59-changes-reported.80x30.txt) | 호스트 보고 자료를 provenance와 함께 유지한다. 검증된 before/written 증거 없이 Recorded로 승격하지 않는다. |
| 60-compaction-retry · 대화 정리 | 재시도 대기 · 2/3 | [보기](plain/60-compaction-retry.80x30.txt) | 타이머가 끝나면 새 정리 요청을 시작한다. 취소 시 pending retry만 해제한다. |
| 61-compaction-complete · 대화 정리 | 완료 · summary 저장됨 | [보기](plain/61-compaction-complete.80x30.txt) | 새 context 관측값이 생긴 뒤만 사용량을 갱신한다. summary 완료와 다음 메시지 실행을 구분한다. |
| 62-answer-review · 답변 검토 | 2/2 답변 작성됨 | [보기](plain/62-answer-review.80x30.txt) | 유효한 request ID에 전체 답변을 한 번 제출한다. 성공/거절/만료 뒤 일반 초안 복원을 분리한다. |
| 63-approval-expired · 승인 요청 | 만료 · 제출 잠금 | [보기](plain/63-approval-expired.80x30.txt) | 폐기된 frame과 request의 선택은 수락하지 않는다. 새 요청은 별도 focus와 기본값으로 표시한다. |
| 64-connect-error · 모델 연결 | 실패 · 인증 코드 만료 | [보기](plain/64-connect-error.80x30.txt) | 새 요청 ID로 인증을 시작한다. 이전 인증 완료 이벤트를 새 연결에 적용하지 않는다. |
| 65-long-command · 승인 상세 | Proposed · 미실행 | [보기](plain/65-long-command.80x30.txt) | 상세 내용을 모두 읽고 Esc로 원래 승인 선택으로 돌아간다. 긴 argv를 줄임표로 제거하지 않는다. |
| 66-streaming · Chat | 응답 받는 중 | [보기](plain/66-streaming.80x30.txt) | 미완료 Markdown을 안전한 텍스트로 보여주고 완료 시 다시 배치한다. 중단 뒤 부분 답변/중단 상태를 보존한다. |

## 요소별 역방향 대조

| 요소 | 화면 witness | 별도 실행 검증 |
|---|---|---|
| T01 · 입력창 구조·placeholder | [01-start](index.html#01-start), [07-chat](index.html#07-chat) | 실제 키/상태 전이와 데이터 보존 확인 |
| T02 · 커서·선택·문자 편집 | [07-chat](index.html#07-chat), [58-editing](index.html#58-editing) | 실제 키/상태 전이와 데이터 보존 확인 |
| T03 · 개행·붙여넣기 | [07-chat](index.html#07-chat), [37-help](index.html#37-help), [44-terminal](index.html#44-terminal), [58-editing](index.html#58-editing) | 실제 키/상태 전이와 데이터 보존 확인 |
| T04 · undo·kill/yank·초안 복원 | [09-queue-paused](index.html#09-queue-paused), [10-queue-edit](index.html#10-queue-edit), [12-history](index.html#12-history), [19-draft-restored](index.html#19-draft-restored), [47-media](index.html#47-media), [50-editor-failed](index.html#50-editor-failed), [58-editing](index.html#58-editing) | 실제 키/상태 전이와 데이터 보존 확인 |
| T05 · 입력 history | [12-history](index.html#12-history) | 실제 키/상태 전이와 데이터 보존 확인 |
| T06 · 대화 찾기 | [13-find](index.html#13-find) | 실제 키/상태 전이와 데이터 보존 확인 |
| T07 · 파일 참조 | [11-references](index.html#11-references) | 실제 키/상태 전이와 데이터 보존 확인 |
| T08 · skill 참조 | [11-references](index.html#11-references) | 실제 키/상태 전이와 데이터 보존 확인 |
| T09 · prompt template | [11-references](index.html#11-references) | 실제 키/상태 전이와 데이터 보존 확인 |
| T10 · 이미지·첨부 입력 | [04-model-failed](index.html#04-model-failed), [11-references](index.html#11-references), [47-media](index.html#47-media) | 실제 키/상태 전이와 데이터 보존 확인 |
| T11 · 외부 편집기 | [50-editor-failed](index.html#50-editor-failed) | 실제 키/상태 전이와 데이터 보존 확인 |
| T12 · Enter·steer | [08-working-queue](index.html#08-working-queue), [09-queue-paused](index.html#09-queue-paused) | 실제 키/상태 전이와 데이터 보존 확인 |
| T13 · queue·편집·재개 | [08-working-queue](index.html#08-working-queue), [09-queue-paused](index.html#09-queue-paused), [10-queue-edit](index.html#10-queue-edit) | 실제 키/상태 전이와 데이터 보존 확인 |
| T14 · interrupt·exit | [08-working-queue](index.html#08-working-queue), [15-stop-approval](index.html#15-stop-approval), [37-help](index.html#37-help), [55-tiny](index.html#55-tiny) | 실제 키/상태 전이와 데이터 보존 확인 |
| T15 · slash palette | [11-references](index.html#11-references), [37-help](index.html#37-help), [56-palette](index.html#56-palette) | 실제 키/상태 전이와 데이터 보존 확인 |
| T16 · 승인 선택 | [14-approval](index.html#14-approval), [15-stop-approval](index.html#15-stop-approval), [55-tiny](index.html#55-tiny), [63-approval-expired](index.html#63-approval-expired), [65-long-command](index.html#65-long-command) | 실제 키/상태 전이와 데이터 보존 확인 |
| T17 · 긴 명령·승인 diff | [14-approval](index.html#14-approval), [16-approval-detail](index.html#16-approval-detail), [23-changes-proposed](index.html#23-changes-proposed), [65-long-command](index.html#65-long-command) | 실제 키/상태 전이와 데이터 보존 확인 |
| T18 · 질문·choice·notes·Previous | [17-question](index.html#17-question), [18-question-choice](index.html#18-question-choice), [19-draft-restored](index.html#19-draft-restored), [48-interview](index.html#48-interview), [57-actions](index.html#57-actions), [62-answer-review](index.html#62-answer-review) | 실제 키/상태 전이와 데이터 보존 확인 |
| T19 · secret 입력·관리 | [20-secret](index.html#20-secret), [21-secret-cleared](index.html#21-secret-cleared), [22-secrets](index.html#22-secrets), [57-actions](index.html#57-actions) | 실제 키/상태 전이와 데이터 보존 확인 |
| T20 · 요청 간 전환·초안 | [09-queue-paused](index.html#09-queue-paused), [10-queue-edit](index.html#10-queue-edit), [12-history](index.html#12-history), [17-question](index.html#17-question), [18-question-choice](index.html#18-question-choice), [19-draft-restored](index.html#19-draft-restored), [20-secret](index.html#20-secret), [36-reconnect](index.html#36-reconnect), [48-interview](index.html#48-interview), [53-reconnect-restored](index.html#53-reconnect-restored), [62-answer-review](index.html#62-answer-review), [63-approval-expired](index.html#63-approval-expired) | 실제 키/상태 전이와 데이터 보존 확인 |
| T21 · assistant streaming | [07-chat](index.html#07-chat), [08-working-queue](index.html#08-working-queue), [52-scrollback](index.html#52-scrollback), [66-streaming](index.html#66-streaming) | 실제 키/상태 전이와 데이터 보존 확인 |
| T22 · tool 실행·접기 | [08-working-queue](index.html#08-working-queue), [26-output](index.html#26-output), [28-results](index.html#28-results), [41-tasks](index.html#41-tasks), [43-mcp](index.html#43-mcp), [46-resources](index.html#46-resources) | 실제 키/상태 전이와 데이터 보존 확인 |
| T23 · 실패·긴 stdout/stderr | [13-find](index.html#13-find), [26-output](index.html#26-output) | 실제 키/상태 전이와 데이터 보존 확인 |
| T24 · reasoning·compaction | [05-settings](index.html#05-settings), [28-results](index.html#28-results), [38-compaction](index.html#38-compaction), [39-compaction-failed](index.html#39-compaction-failed), [60-compaction-retry](index.html#60-compaction-retry), [61-compaction-complete](index.html#61-compaction-complete), [66-streaming](index.html#66-streaming) | 실제 키/상태 전이와 데이터 보존 확인 |
| T25 · Markdown 문서 계층 | [07-chat](index.html#07-chat), [45-documents](index.html#45-documents), [66-streaming](index.html#66-streaming) | 실제 키/상태 전이와 데이터 보존 확인 |
| T26 · 코드·syntax·공백 | [26-output](index.html#26-output), [27-source](index.html#27-source), [45-documents](index.html#45-documents), [66-streaming](index.html#66-streaming) | 실제 키/상태 전이와 데이터 보존 확인 |
| T27 · 표·차트·도식 | [45-documents](index.html#45-documents) | 실제 키/상태 전이와 데이터 보존 확인 |
| T28 · 링크·copy·export | [26-output](index.html#26-output), [27-source](index.html#27-source), [28-results](index.html#28-results), [33-rescue](index.html#33-rescue), [34-export-error](index.html#34-export-error), [42-memory](index.html#42-memory) | 실제 키/상태 전이와 데이터 보존 확인 |
| T29 · 본문 이미지·media | [47-media](index.html#47-media) | 실제 키/상태 전이와 데이터 보존 확인 |
| T30 · 파일 diff 표시 | [16-approval-detail](index.html#16-approval-detail), [23-changes-proposed](index.html#23-changes-proposed), [24-changes-recorded](index.html#24-changes-recorded), [25-changes-current](index.html#25-changes-current), [28-results](index.html#28-results), [59-changes-reported](index.html#59-changes-reported) | 실제 키/상태 전이와 데이터 보존 확인 |
| T31 · Changes view | [16-approval-detail](index.html#16-approval-detail), [23-changes-proposed](index.html#23-changes-proposed), [24-changes-recorded](index.html#24-changes-recorded), [25-changes-current](index.html#25-changes-current), [59-changes-reported](index.html#59-changes-reported) | 실제 키/상태 전이와 데이터 보존 확인 |
| T32 · Output view | [13-find](index.html#13-find), [26-output](index.html#26-output) | 실제 키/상태 전이와 데이터 보존 확인 |
| T33 · Transcript view | [32-diagnostics](index.html#32-diagnostics) | 실제 키/상태 전이와 데이터 보존 확인 |
| T34 · Request view | [32-diagnostics](index.html#32-diagnostics) | 실제 키/상태 전이와 데이터 보존 확인 |
| T35 · 모델 picker | [03-model](index.html#03-model), [04-model-failed](index.html#04-model-failed), [51-picker-stale](index.html#51-picker-stale) | 실제 키/상태 전이와 데이터 보존 확인 |
| T36 · 새 대화·resume picker | [29-sessions](index.html#29-sessions), [30-session-error](index.html#30-session-error), [31-fork](index.html#31-fork), [35-storage](index.html#35-storage), [54-empty-list](index.html#54-empty-list) | 실제 키/상태 전이와 데이터 보존 확인 |
| T37 · tree·fork picker | [31-fork](index.html#31-fork) | 실제 키/상태 전이와 데이터 보존 확인 |
| T38 · help·status·usage | [32-diagnostics](index.html#32-diagnostics), [37-help](index.html#37-help), [56-palette](index.html#56-palette), [57-actions](index.html#57-actions) | 실제 키/상태 전이와 데이터 보존 확인 |
| T39 · 빈/loading/error/disabled/stale | [01-start](index.html#01-start), [02-connect](index.html#02-connect), [03-model](index.html#03-model), [04-model-failed](index.html#04-model-failed), [06-settings-failed](index.html#06-settings-failed), [21-secret-cleared](index.html#21-secret-cleared), [22-secrets](index.html#22-secrets), [25-changes-current](index.html#25-changes-current), [29-sessions](index.html#29-sessions), [30-session-error](index.html#30-session-error), [35-storage](index.html#35-storage), [36-reconnect](index.html#36-reconnect), [39-compaction-failed](index.html#39-compaction-failed), [43-mcp](index.html#43-mcp), [51-picker-stale](index.html#51-picker-stale), [53-reconnect-restored](index.html#53-reconnect-restored), [54-empty-list](index.html#54-empty-list), [60-compaction-retry](index.html#60-compaction-retry), [63-approval-expired](index.html#63-approval-expired), [64-connect-error](index.html#64-connect-error) | 실제 키/상태 전이와 데이터 보존 확인 |
| T40 · 공통 selection panel | [03-model](index.html#03-model), [11-references](index.html#11-references), [12-history](index.html#12-history), [14-approval](index.html#14-approval), [17-question](index.html#17-question), [29-sessions](index.html#29-sessions), [30-session-error](index.html#30-session-error), [31-fork](index.html#31-fork), [51-picker-stale](index.html#51-picker-stale), [54-empty-list](index.html#54-empty-list), [56-palette](index.html#56-palette) | 실제 키/상태 전이와 데이터 보존 확인 |
| T41 · header·실행 identity | [01-start](index.html#01-start), [07-chat](index.html#07-chat) | 실제 키/상태 전이와 데이터 보존 확인 |
| T42 · 상태줄·진행·대기 | [02-connect](index.html#02-connect), [08-working-queue](index.html#08-working-queue), [36-reconnect](index.html#36-reconnect), [38-compaction](index.html#38-compaction), [41-tasks](index.html#41-tasks), [53-reconnect-restored](index.html#53-reconnect-restored), [60-compaction-retry](index.html#60-compaction-retry), [64-connect-error](index.html#64-connect-error), [66-streaming](index.html#66-streaming) | 실제 키/상태 전이와 데이터 보존 확인 |
| T43 · footer·키 힌트 | [07-chat](index.html#07-chat), [08-working-queue](index.html#08-working-queue), [15-stop-approval](index.html#15-stop-approval), [18-question-choice](index.html#18-question-choice), [37-help](index.html#37-help), [55-tiny](index.html#55-tiny), [57-actions](index.html#57-actions), [62-answer-review](index.html#62-answer-review), [65-long-command](index.html#65-long-command) | 실제 키/상태 전이와 데이터 보존 확인 |
| T44 · 레이아웃·공간 배분 | [52-scrollback](index.html#52-scrollback), [55-tiny](index.html#55-tiny) | PTY resize·공개 frame·selection/source anchor 보존 |
| T45 · focus·cursor·공개된 frame | [14-approval](index.html#14-approval), [16-approval-detail](index.html#16-approval-detail), [36-reconnect](index.html#36-reconnect), [51-picker-stale](index.html#51-picker-stale), [53-reconnect-restored](index.html#53-reconnect-restored), [55-tiny](index.html#55-tiny), [63-approval-expired](index.html#63-approval-expired) | PTY resize·공개 frame·selection/source anchor 보존 |
| T46 · resize·reflow·위치 유지 | [16-approval-detail](index.html#16-approval-detail), [23-changes-proposed](index.html#23-changes-proposed), [24-changes-recorded](index.html#24-changes-recorded), [52-scrollback](index.html#52-scrollback), [59-changes-reported](index.html#59-changes-reported), [65-long-command](index.html#65-long-command) | PTY resize·공개 frame·selection/source anchor 보존 |
| T47 · inline/fullscreen·publication | [44-terminal](index.html#44-terminal), [49-preview](index.html#49-preview), [52-scrollback](index.html#52-scrollback) | PTY resize·공개 frame·selection/source anchor 보존 |
| T48 · 스크롤·follow latest | [13-find](index.html#13-find), [52-scrollback](index.html#52-scrollback) | PTY resize·공개 frame·selection/source anchor 보존 |
| T49 · 항목 접기와 전체 접기 | [24-changes-recorded](index.html#24-changes-recorded), [52-scrollback](index.html#52-scrollback) | 실제 키/상태 전이와 데이터 보존 확인 |
| T50 · theme·semantic colors | [05-settings](index.html#05-settings), [06-settings-failed](index.html#06-settings-failed), [44-terminal](index.html#44-terminal) | 실제 터미널·폰트·SSH/tmux·비시각 경로 확인 |
| T51 · glyph·Unicode·폭 | [44-terminal](index.html#44-terminal), [58-editing](index.html#58-editing) | 실제 터미널·폰트·SSH/tmux·비시각 경로 확인 |
| T52 · motion·attention·notification | [05-settings](index.html#05-settings), [06-settings-failed](index.html#06-settings-failed), [44-terminal](index.html#44-terminal) | 실제 키/상태 전이와 데이터 보존 확인 |
| T53 · mouse·selection·clipboard | [20-secret](index.html#20-secret), [27-source](index.html#27-source), [34-export-error](index.html#34-export-error), [44-terminal](index.html#44-terminal) | 실제 터미널·폰트·SSH/tmux·비시각 경로 확인 |
| T54 · 키 protocol·충돌·대체 경로 | [37-help](index.html#37-help), [44-terminal](index.html#44-terminal), [57-actions](index.html#57-actions) | 실제 터미널·폰트·SSH/tmux·비시각 경로 확인 |
| T55 · 종료·suspend·복귀 | [44-terminal](index.html#44-terminal), [50-editor-failed](index.html#50-editor-failed) | 실제 터미널·폰트·SSH/tmux·비시각 경로 확인 |
| T56 · SSH/tmux·image protocol | [34-export-error](index.html#34-export-error), [44-terminal](index.html#44-terminal), [47-media](index.html#47-media) | 실제 터미널·폰트·SSH/tmux·비시각 경로 확인 |
| T57 · 비시각 접근·읽기/내보내기 | [05-settings](index.html#05-settings), [27-source](index.html#27-source), [33-rescue](index.html#33-rescue), [34-export-error](index.html#34-export-error), [44-terminal](index.html#44-terminal) | 실제 터미널·폰트·SSH/tmux·비시각 경로 확인 |
| T58 · 화면 응답성·용량 | [08-working-queue](index.html#08-working-queue), [41-tasks](index.html#41-tasks), [52-scrollback](index.html#52-scrollback) | 실제 큰 출력에서 frame latency·idle wake·취소 지연 측정; 이 예시는 성능 근거 아님 |
| T59 · custom renderer·document·status | [40-plan](index.html#40-plan), [41-tasks](index.html#41-tasks), [42-memory](index.html#42-memory), [43-mcp](index.html#43-mcp), [45-documents](index.html#45-documents), [46-resources](index.html#46-resources) | custom renderer 오류·source fallback·호스트별 문서 shape 검증 |
| T60 · 화면 간 전이·목록 누락 감사 | [32-diagnostics](index.html#32-diagnostics), [37-help](index.html#37-help), [49-preview](index.html#49-preview), [56-palette](index.html#56-palette) | 실제 키/상태 전이와 데이터 보존 확인 |
| T61 · offline preview 화면 | [49-preview](index.html#49-preview) | 실제 키/상태 전이와 데이터 보존 확인 |
| T62 · 수동 compaction | [38-compaction](index.html#38-compaction), [39-compaction-failed](index.html#39-compaction-failed), [60-compaction-retry](index.html#60-compaction-retry), [61-compaction-complete](index.html#61-compaction-complete) | 실제 키/상태 전이와 데이터 보존 확인 |
| T63 · interview 작업 사본 lifecycle | [48-interview](index.html#48-interview) | 실제 키/상태 전이와 데이터 보존 확인 |
| T64 · 저장 실패·시작 안내 | [01-start](index.html#01-start), [02-connect](index.html#02-connect), [30-session-error](index.html#30-session-error), [33-rescue](index.html#33-rescue), [34-export-error](index.html#34-export-error), [35-storage](index.html#35-storage), [36-reconnect](index.html#36-reconnect), [64-connect-error](index.html#64-connect-error) | 실제 키/상태 전이와 데이터 보존 확인 |
| T22.a · 계획 단계·진행 중/대기/완료·빈 plan | [28-results](index.html#28-results), [40-plan](index.html#40-plan) | 실제 키/상태 전이와 데이터 보존 확인 |
| T22.b · notice/warning·보고된 Turn duration | [28-results](index.html#28-results), [33-rescue](index.html#33-rescue), [38-compaction](index.html#38-compaction), [61-compaction-complete](index.html#61-compaction-complete) | 실제 키/상태 전이와 데이터 보존 확인 |
| T22.c · resource link 카드·URI·unknown metadata·크기 초과 | [43-mcp](index.html#43-mcp), [46-resources](index.html#46-resources) | 실제 키/상태 전이와 데이터 보존 확인 |
| T22.d · embedded text/blob/image·metadata·잘못된 shape | [46-resources](index.html#46-resources) | 실제 키/상태 전이와 데이터 보존 확인 |
| T22.e · batch file reads·range·empty·개별 실패·이어 읽기 | [46-resources](index.html#46-resources) | 실제 키/상태 전이와 데이터 보존 확인 |
| T22.f · directory listing·부분 목록·빈/미완전 entry | [46-resources](index.html#46-resources) | 실제 키/상태 전이와 데이터 보존 확인 |
| T22.g · file/content search·match cap·잘못된 metadata | [46-resources](index.html#46-resources) | 실제 키/상태 전이와 데이터 보존 확인 |
| T22.h · execution details·host/call/outcome·retained output | [26-output](index.html#26-output), [46-resources](index.html#46-resources) | 실제 키/상태 전이와 데이터 보존 확인 |
| T25.a · Proposed plan·host/help/status·terminal input 문서 | [40-plan](index.html#40-plan), [42-memory](index.html#42-memory), [45-documents](index.html#45-documents) | 실제 키/상태 전이와 데이터 보존 확인 |
| T29.a · audio·unknown media fallback | [47-media](index.html#47-media) | 실제 키/상태 전이와 데이터 보존 확인 |

## 반드시 실제 상호작용으로 보강할 검수

1. 입력 전송 수락/거절, queued/steer unsupported, 질문 Previous/최종 제출/만료, 일반·질문·비밀 draft 소유권.
2. 승인 상세 왕복/resize/late completion, 재사용 범위의 host 검증, 미지원 거절에서 턴 중단.
3. 이미지 준비 취소·marker undo·모델 전환, grapheme/emoji/ZWJ/IME와 여러 줄 paste, 외부 편집/빈 결과/suspend 왕복.
4. transcript follow/source anchor, 항목별/전체 접기, 표/도식/source copy, partial/잘못된 typed payload.
5. resume empty/loading/all-disabled/stale/error, compaction pending/running/cancel/fail/complete, queue edit/remove/new topic.
6. 저장 용량/실패와 메모리 suffix rescue, export 범위/실패/clipboard 미확인, reconnect failed/unknown/recovered/중복 억제.
7. 100→80→40→24→80 resize와 작은 높이, dark/light/mono, SSH/tmux 및 reduced motion, 큰 기록 응답성.

현재 장면은 대표 정지 상태다. 39/60/61번은 정리 실패/재시도/완료를 별도로 보여준다.
51번의 empty/all-disabled 문구는 안내이며, 모든 picker 상태를 실제 재현한 것으로 집계하지 않는다.
58번은 selection/caret의 표기 예시이며 실제 cursor 이동/선택 렌더링 검증이 아니다.
런타임 없는 정적 예시로 이 목록을 완료 처리하지 않는다.
