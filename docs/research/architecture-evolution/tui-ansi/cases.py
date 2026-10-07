"""Research fixtures, not commands or implemented Yo behavior. All values synthetic."""
CASES = []
def case(id, title, state, focus, actions, coverage, refs, body, after):
    CASES.append(dict(id=id, title=title, state=state, focus=focus, actions=actions,
                      coverage=coverage.split(), refs=refs.split(), body=body.strip().splitlines(), after=after,
                      status='proposed', keys='proposed unless explicitly listed in README'))

case('01-start', '시작', '연결된 모델 없음', '선택: 모델 연결', ['Enter 연결', 'Esc 종료'], 'T01 T39 T41 T64', 'P02 C01', '''
# yo · 코드 작업을 시작하세요
프로젝트  ~/work/parser
읽은 지침  AGENTS.md · 프로젝트 범위
! 모델 연결이 필요합니다.
> 모델 연결과 기본 모델 선택
  이전 작업 찾아보기
연결 후 이 프로젝트에서 첫 질문을 작성합니다.
''', '연결 완료 후 빈 일반 초안으로 돌아온다. 연결 취소는 설정을 변경하지 않는다.')
case('02-connect', '모델 연결', '연결 대기 · 00:24', '브라우저에서 인증', ['Esc 연결 취소'], 'T39 T42 T64', 'P03 C08', '''
# 예시 제공자 계정 연결
1. 브라우저에서 로그인합니다.
2. 승인이 끝나면 이 화면이 갱신됩니다.
기기 코드  DEMO-ONLY
인증 URL  https://example.invalid/device
! 코드 만료 시 새 연결을 시작하세요.
비밀 토큰은 대화 기록에 저장하지 않습니다.
''', '성공 시 모델 목록. 만료/거절 시 이유와 재연결 행동. 대화 초안은 보존한다.')
case('03-model', '모델 선택', '검색: code · 2개', '선택: code-small', ['Enter 다음 턴 예약', 'Esc 돌아가기'], 'T35 T39 T40', 'P05 C08', '''
# 현재와 다음 턴을 구분합니다
현재  native / code-large
기본값  code-large · 새 대화에 적용
> code-small · 사용 가능
  code-vision · 사용 불가: 계정 연결 필요
예약  없음
실행 중 모델을 바꾸면 다음 턴부터 적용합니다.
이미지 첨부가 있으면 입력 지원 여부를 먼저 확인합니다.
''', '수락 후 예약 상태를 표시하고 실제 전환 성공 때 현재 모델을 바꾼다. 실패 시 현재 모델/첨부를 보존한다.')
case('04-model-failed', '모델 전환', '실패 · 현재 모델 유지', '이미지 1개 보관 중', ['Esc 초안으로'], 'T10 T35 T39', 'P05 C01', '''
! code-small은 이미지 입력을 지원하지 않습니다.
현재  code-vision
예약  적용되지 않음
초안  이 화면의 오류를 설명해줘
첨부  screenshot.png · 준비 완료
> 이미지 지원 모델 선택
  첨부를 명시적으로 제거한 뒤 다시 선택
모델 변경만으로 첨부를 버리지 않습니다.
''', '모델 재선택 또는 첨부 제거를 별도 행동으로 수행한다. 자동 재전송하지 않는다.')
case('05-settings', '화면 설정', '적용 범위: 현재 세션', '선택: 움직임 줄이기', ['Enter 켜기', 'Esc 돌아가기'], 'T24 T50 T52 T57', 'P06 C08', '''
# 보기와 모델 설정은 다릅니다
  테마                 터미널 기본
> 움직임 줄이기        꺼짐
  이미지 본문 표시     켜짐
  공개 reasoning 표시  꺼짐
  새 세션 기본값 저장  별도 행동
이미지 표시를 꺼도 첨부 전송 기능은 바뀌지 않습니다.
reasoning 표시와 모델의 사고 설정은 별개입니다.
''', '현재 보기 변경과 기본값 저장을 분리한다. 저장 실패 시 현재 값과 미저장 상태를 함께 표시한다.')
case('06-settings-failed', '화면 설정', '기본값 저장 실패', '현재 세션에는 적용됨', ['Esc 돌아가기'], 'T39 T50 T52', 'P06 C08', '''
! 설정 파일에 쓸 수 없습니다.
현재 움직임 줄이기  켜짐
새 세션 기본값      꺼짐
> 저장 다시 시도
  현재 세션에서만 사용
실패한 위치  ~/.config/yo/settings
권한을 확인한 뒤 다시 저장할 수 있습니다.
''', '재시도 전 현재 설정을 잃지 않는다. 실패를 성공 알림으로 바꾸지 않는다.')
case('07-chat', 'Chat', '대기 · 입력 가능', '일반 초안 · 2줄', ['Enter 보내기', 'Ctrl+G 외부 편집'], 'T01 T02 T03 T21 T25 T41 T43', 'P01 P02 P11 C01 C02', '''
# 사용자
parser의 빈 입력 처리를 고쳐줘.
# yo
입력 경계와 기존 테스트를 먼저 확인하겠습니다.
  [완료] src/parser.rs 읽기
  [완료] tests/parser.rs 읽기
# 일반 초안
> 빈 문자열과 한글 입력도 검사해줘.
> 기존 공개 API는 유지해줘.
모델  native / code-large
프로젝트  ~/work/parser · fix/empty-input
''', 'Enter는 일반 입력 수락 요청이다. 수락 확인 전에 초안을 완료된 메시지로 취급하지 않는다.')
case('08-working-queue', 'Chat', '실행 중 · 00:18', '다음 메시지 2개', ['Esc 중단', 'Ctrl+C 중단'], 'T12 T13 T14 T21 T22 T42 T43 T58', 'P03 P04 C03 C04', '''
# 빈 입력 처리 수정
[실행 중] cargo test -p parser
  현재 실행에 전달: 경계값도 확인해줘
  다음 턴 #q2: 변경 근거를 정리해줘
  다음 턴 #q3: README 예시도 점검해줘
[일반 초안 보관] 성능 영향도 알려줘
현재 모델은 실행 중 메시지 전달을 지원합니다.
진행 중 전달 실패는 다음 턴 메시지와 구분합니다.
새 출력 12줄 · 현재 읽는 위치 유지
''', '중단은 현재 턴에 요청한다. 대기 메시지를 자동 삭제하지 않는다. 큐 편집은 항목 ID와 첨부를 보존한다.')
case('09-queue-paused', '대기 메시지', '일시 정지 · 2개', '선택: #q2 · 다음 턴', ['Enter 상세 보기', 'Esc 초안으로'], 'T04 T12 T13 T20', 'P04 C04', '''
> #q2 변경 근거를 정리해줘
  첨부: @src/parser.rs · 참조 1개
  #q3 README 예시도 점검해줘
일반 초안  성능 영향도 알려줘
# 항목별 행동
편집 / 삭제 / 순서 조정 / 대기열 재개
새 대화 시작 전 대기열을 유지할지 선택합니다.
편집 시 여러 메시지를 하나의 텍스트로 합치지 않습니다.
''', '선택 항목 전용 편집으로 이동한다. 취소 시 원래 일반 초안으로 돌아온다. 삭제/재개는 별도 명시 행동.')
case('10-queue-edit', '메시지 편집', '대기열 일시 정지', '편집: #q2', ['Enter 편집 저장', 'Esc 편집 취소'], 'T04 T13 T20', 'P04 C04', '''
# 다음 턴 메시지 #q2
> 변경 근거와 테스트 결과를 정리해줘
참조  @src/parser.rs · snapshot 보존
일반 초안  성능 영향도 알려줘 · 별도 보관
다른 메시지 #q3은 수정하지 않습니다.
저장은 실행 재개와 별개입니다.
''', '같은 #q2 내용만 교체하고 일시 정지를 유지한다. 취소 시 편집 전 메시지로 복원한다.')
case('11-references', '참조 선택', '검색: @parser', '선택: src/parser.rs', ['Enter 참조 삽입', 'Esc 검색 닫기'], 'T07 T08 T09 T10 T15 T40', 'P01 C01', '''
# 프로젝트 파일
> src/parser.rs
  tests/parser.rs
# 사용 가능한 참조
skill  review · project / .agents/skills
skill  review · user / ~/.agents/skills
prompt  explain-change · 일반 텍스트 삽입
첨부  screenshot.png · 준비 중 1/1
! 준비 중 첨부가 있으면 전송 전 완료를 확인합니다.
''', '참조 snapshot을 초안에 삽입한다. 같은 이름의 skill은 source/scope로 구별한다. template 결과를 자동 명령 실행하지 않는다.')
case('12-history', '입력 기록', '검색: 경계값', '일반 초안 따로 보관', ['Enter 불러오기', 'Esc 검색 취소'], 'T04 T05 T20 T40', 'P07 C07', '''
> 경계값 테스트를 추가해줘
  어제 · 참조 1개
  경계값 실패 원인을 설명해줘
  3일 전 · 참조 없음
현재 초안  성능 영향도 알려줘
불러온 내용은 보내기 전에 편집합니다.
취소하면 현재 초안과 커서로 돌아옵니다.
''', '기록은 편집 가능한 입력으로만 불러온다. 자동 제출하지 않는다.')
case('13-find', '대화 찾기', '범위: 대화 + 도구', '검색: empty · 3건', ['Enter 결과로 이동', 'Esc 찾기 닫기'], 'T06 T23 T32 T48', 'P10 C10', '''
> 도구 #call7 · tests/parser.rs:42
  assertion failed: parse_empty
  사용자 · 빈 입력도 검사해줘
  답변 · empty input은 빈 결과를 반환합니다.
공개된 대화와 보관된 도구 출력에서 검색합니다.
버려진 출력/비밀 입력은 검색 대상이 아닙니다.
색인 갱신 중에는 마지막 결과 시각을 표시합니다.
''', '결과의 항목과 source 위치로 이동한다. 늦은 검색 결과가 새 질의를 덮지 않는다.')
case('14-approval', '승인 요청', 'Proposed · 미실행', '선택: 이번 요청 거절', ['Enter 거절', 'Esc 거절'], 'T16 T17 T40 T45', 'P08 P09 C05', '''
# 실행 효과 확인
호스트  local · ~/work/parser
프로그램  cargo
인수  ["test", "-p", "parser"]
프로필  workspace-test · 네트워크 차단
> 이번 요청 거절
  이번 1회 허용
  이 작업의 동일 범위 허용
범위 재사용은 실제 실행 제한이 가능한 경우에만 제공됩니다.
전체 명령과 환경 보기 · 별도 상세 화면
''', '선택한 요청 ID/공개 frame이 유효할 때만 응답한다. 표시한 실행 범위 이상으로 권한을 확대하지 않는다.')
case('15-stop-approval', '승인 요청', 'Proposed · 미실행', '요청 거절 기능 없음', ['Esc 턴 중단', 'Ctrl+C 턴 중단'], 'T14 T16 T43', 'C05', '''
# 외부 호스트 실행 요청
요청 ID  req-07
이 호스트는 개별 요청 거절을 지원하지 않습니다.
이번 1회 허용 · 명령 전체를 확인한 뒤 선택
! 취소하려면 현재 턴을 중단합니다.
거절과 턴 중단을 같은 이름으로 표시하지 않습니다.
''', 'Esc는 Decline 응답을 만들지 않고 턴 중단 요청을 보낸다. 지연된 응답은 폐기한다.')
case('16-approval-detail', '승인 상세', 'Proposed · 미실행', '복귀 선택: 이번 요청 거절', ['Esc 승인으로 복귀'], 'T17 T30 T31 T45 T46', 'C05 C09', '''
# 전체 실행 내용
호스트  local
cwd  /work/parser
executable  /usr/bin/cargo
argv[0]  test
argv[1]  -p
argv[2]  parser
환경 프로필  workspace-test
네트워크  차단
쓰기 범위  /work/parser/target
# 제안 파일 변경 · 아직 미적용
src/parser.rs
-  return Err(EmptyInput);
+  return Ok(Vec::new());
상세 화면에서는 승인하지 않습니다.
''', '읽던 상세 위치를 보관하고 동일한 요청의 선택으로 돌아간다. 만료된 요청이면 수락을 비활성화한다.')
case('17-question', '질문 1/2', '답변 입력 · 요청 #r8', '일반 초안 1개 보관', ['Enter 답변 저장', 'Esc 턴 중단'], 'T18 T20 T40', 'P09 C06', '''
# 빈 입력의 결과는 무엇이어야 하나요?
> 빈 결과 반환
  오류 반환
# 이 질문의 메모
> 현재 API와 일관되게 해주세요.
# 보관한 일반 초안 · 답변으로 전송 안 함
성능 영향도 알려줘 · 참조 1개
이전 질문 / 다음 질문 / 전체 답변 검토
현재 선택과 메모는 이 요청에만 속합니다.
''', '답변 저장 후 질문 2/2로 이동한다. 모든 답변 검토 후 전체 제출. 일반 초안은 질문 종료 시 복원한다.')
case('18-question-choice', '질문 2/2', '메모 미지원', '선택: 테스트 추가', ['Enter 답변 저장', 'Esc 턴 중단'], 'T18 T20 T43', 'C06', '''
# 회귀 테스트를 추가할까요?
> 테스트 추가
  현재 테스트만 실행
Tab  다음 선택으로 이동 · 제출하지 않음
이전 질문으로 돌아가 답변을 수정할 수 있습니다.
일반 초안은 계속 별도 보관 중입니다.
''', 'Tab은 선택 이동만 한다는 제안. 실제 구현 전 request capability와 dispatch를 함께 변경해야 한다.')
case('19-draft-restored', 'Chat', '질문 완료 · 입력 복원', '일반 초안 · 참조 1개', ['Enter 보내기', 'Ctrl+G 외부 편집'], 'T04 T18 T20', 'C01 C06', '''
[완료] 질문 2개에 답변했습니다.
답변 요약  빈 결과 반환 / 테스트 추가
# 일반 초안
> 성능 영향도 알려줘
참조  @src/parser.rs · 기존 snapshot
질문에 쓴 메모는 일반 초안에 합치지 않습니다.
입력 위치와 첨부가 질문 전 상태로 돌아왔습니다.
''', '복원된 일반 초안은 사용자가 보낼 때만 새 입력이 된다.')
case('20-secret', '비밀 입력', '입력됨 · 길이 비공개', '요청: 예시 API 토큰', ['Enter 비밀 전달', 'Esc 입력 취소'], 'T19 T20 T53', 'C06', '''
# 요청한 호스트  local
값  [입력됨]
전체 지우기 · 명시적인 별도 행동
여러 줄/커서 위치와 관계없이 전체 값을 지웁니다.
길이와 실제 문자는 화면/기록에 표시하지 않습니다.
저장된 복구 값 삭제는 별도 관리 화면에서 합니다.
일반 초안은 보관 중입니다.
''', '명시적 전체 지우기 뒤 Not entered 상태로 전환한다. 이 예시 파일에는 실제 토큰/가짜 토큰 바이트도 없다.')
case('21-secret-cleared', '비밀 입력', '미입력 · 전체 지움', '제출 불가: 값 없음', ['Esc 입력 취소'], 'T19 T39', 'C06', '''
# 예시 API 토큰
값  [미입력]
[완료] 임시 입력의 모든 줄을 지웠습니다.
Enter는 값을 제출하지 않습니다.
새 값을 입력하거나 요청을 취소하세요.
저장된 복구 값은 이 행동으로 삭제되지 않습니다.
''', '입력이 없을 때 제출을 막는다. 새 값을 입력하면 입력됨 상태로 전환한다.')
case('22-secrets', '비밀 관리', '값은 표시하지 않음', '선택: example-api', ['Enter 메타데이터', 'Esc 돌아가기'], 'T19 T39', 'C06', '''
> example-api · local · 저장된 복구 값 있음
  project-key · 이 세션 · 복구 값 없음
# 선택 항목
사용 위치  모델 연결
마지막 갱신  2026-10-04 · 예시
복구 값 삭제 · 별도 확인 필요
현재 비밀 입력을 지우는 행동과 다릅니다.
''', '관리 화면은 metadata만 보여준다. 삭제 대상과 영향을 확인한 뒤 지정된 저장 값만 제거한다.')
case('23-changes-proposed', 'Changes', 'Proposed · 미적용', 'src/parser.rs · +1 -1', ['Esc 승인으로 복귀'], 'T17 T30 T31 T46', 'C05 C09', '''
# 제안 변경 · 아직 파일에 쓰지 않음
@@ parse_empty @@
-    return Err(EmptyInput);
+    return Ok(Vec::new());
원문 들여쓰기와 줄바꿈은 별도 원문에 보존됩니다.
파일 1/2 · 다음: tests/parser.rs
현재 Git diff와 이 제안은 서로 다른 자료입니다.
''', '복귀 시 원래 승인 요청과 선택을 유지한다. 여기에서 Recorded로 바꾸지 않는다.')
case('24-changes-recorded', 'Changes', 'Recorded · 실행 기록', 'src/parser.rs · +1 -1', ['Esc 대화로 복귀'], 'T30 T31 T46 T49', 'C09', '''
# 도구 #call9가 보고한 변경
@@ parse_empty @@
-    return Err(EmptyInput);
+    return Ok(Vec::new());
기록 시각  13:42:10 · 예시
이후 파일이 바뀌었을 수 있습니다.
현재 작업 폴더 확인은 별도 행동입니다.
''', '실행 시점의 기록을 유지한다. 현재 파일 상태와 일치한다고 추정하지 않는다.')
case('25-changes-current', '작업 폴더', 'Current · 지금 조회', 'Git diff · tracked 파일', ['Esc 결과로 복귀'], 'T30 T31 T39', 'C09', '''
# 현재 작업 폴더 조회 · 제안 화면
조회 시각  13:45:00 · 예시
src/parser.rs  +1 -1
tests/parser.rs  +8 -0
미추적 파일  이 diff에 포함되지 않음
기록된 도구 변경과 다르면 차이를 명시합니다.
이 화면만으로 테스트 성공을 의미하지 않습니다.
''', '새 조회에서만 갱신한다. Proposed/Recorded 증거를 덮어쓰지 않는다.')
case('26-output', 'Output', '실패 · exit 1', '선택: #call7 · 부분 출력', ['Esc 대화로 복귀'], 'T22 T23 T26 T28 T32 T22.h', 'P10 C10', '''
# cargo test -p parser
호스트  local · cwd ~/work/parser
보관 범위  첫 2 KiB + 마지막 6 KiB
! 가운데 40 KiB는 보관되지 않았습니다.
stdout:
  running 12 tests
stderr:
  assertion failed: parse_empty
  tests/parser.rs:42
실패 원문 찾기 / 보관된 원문 복사
전체 출력 복구가 가능한 것처럼 안내하지 않습니다.
''', '선택한 call의 보관된 원문 위치를 연다. /output 최신 항목과 선택 항목 접근을 구별하는 제안.')
case('27-source', '원문 보기', '보관된 원문 · 수정 없음', '복사 범위: 선택 코드', ['Esc 출력으로 복귀'], 'T26 T28 T53 T57', 'P10 C10 C11', '''
# src/parser.rs · UTF-8
fn parse(input: &str) -> Vec<Token> {
    if input.is_empty() {
        return Vec::new();
    }
    tokenize(input)
}
표시용 줄바꿈은 복사 원문에 넣지 않습니다.
줄 번호/장식/ANSI 색상은 복사 대상에서 제외합니다.
복사 요청 후 터미널 수신 여부는 별도로 알 수 없습니다.
''', '원문 바이트를 유지해 복사를 요청한다. 화면의 줄바꿈을 원문 줄바꿈으로 취급하지 않는다.')
case('28-results', '작업 결과', '완료 · 예시 증거', '선택: 테스트 결과', ['Enter 결과 상세', 'Esc 대화로 복귀'], 'T22 T24 T28 T30 T22.a T22.b', 'P11 P12 C09 C10', '''
# 빈 입력 처리 수정
[완료] 경계 조건 확인
[완료] 구현과 회귀 테스트 수정
[완료] cargo test -p parser · 12 passed
> 테스트 결과 · #call12 · exit 0
  변경 기록 · 파일 2개 · Recorded
! 전체 workspace 테스트는 실행하지 않았습니다.
작업 시간  02:14 · 예시 값
다음 행동  현재 Git diff 검토
''', '각 요약은 실제 call/변경 기록 상세로 연결해야 한다. 이 fixture의 성공 수치는 실행 결과가 아닌 합성 값이다.')
case('29-sessions', '이전 작업', '이 프로젝트 · 검색: parser', '선택: 빈 입력 처리', ['Enter 재개', 'Esc 초안으로'], 'T36 T39 T40', 'P07 C07', '''
> 빈 입력 처리 · 15분 전
  마지막 입력: 빈 문자열도 검사해줘
  모델: native / code-large · 8개 메시지
  ID: session-demo-21
  토큰 위치 추적 · 어제
  마지막 입력: 줄 번호 오류를 확인해줘
2 / 86개 · 다음 페이지
전체 프로젝트 / 미리보기 / 이름 변경
''', '선택 ID의 재개가 성공한 후에만 현재 세션을 전환한다. 실패/취소 시 검색과 일반 초안을 복원한다.')
case('30-session-error', '이전 작업', '재개 실패 · 현재 세션 유지', '초안과 검색어 보관', ['Esc 목록으로'], 'T36 T39 T40 T64', 'P07 C07', '''
! 선택한 기록을 읽을 수 없습니다.
작업  빈 입력 처리 · session-demo-21
원인  기록 파일 접근 거절
> 다시 읽기
  진단 상세 보기
  목록으로 돌아가기
현재 초안을 새 세션으로 자동 전송하지 않습니다.
''', '오류 뒤에도 원래 picker의 선택과 검색을 유지한다.')
case('31-fork', '분기 시작점', '상속 범위 확인', '선택: 사용자 입력 #4', ['Enter 분기 생성', 'Esc 돌아가기'], 'T36 T37 T40', 'P07 C07', '''
# 빈 입력 처리 · session-demo-21
  #2 기존 parser 설명
> #4 빈 문자열도 검사해줘
  #6 회귀 테스트 결과
상속  선택 지점 이전의 공개 대화와 유효한 상태
제외  이후 실행 결과
부모 기록은 변경하지 않습니다.
진행 중 요청이 있는 지점은 선택할 수 없습니다.
''', '선택한 anchor 기준으로 새 세션을 만든다. 불가능한 지점은 이유를 보이고 수락하지 않는다.')
case('32-diagnostics', '진단', 'Transcript / Request', '사용자 결과와 별도', ['Esc 대화로 복귀'], 'T33 T34 T38 T60', 'C02 C10', '''
# Transcript · F2
의미 기록과 상태 전이를 확인합니다.
# Request · F3
요청 ID, 상관관계, 전달 상태를 확인합니다.
Request는 승인 대기함이 아닙니다.
도구 stdout/stderr는 Output에서 확인합니다.
파일 변경은 Changes에서 확인합니다.
# 현재 상태
host local · model code-large · queue 2
context 사용량: 알 수 없음
''', '상세 진단과 사용자 결과 탐색을 분리한다. 알 수 없는 사용량을 0으로 표시하지 않는다.')
case('33-rescue', '기록 저장 실패', 'History not saved', '대화 계속 가능 · 저장 공백', ['Esc 대화로 복귀'], 'T28 T57 T64 T22.b', 'P04 C11', '''
! 디스크 쓰기에 실패했습니다.
기록 파일  마지막 저장 이후 3개 항목 누락
메모리  공개 대화 일부와 일반 초안 보관 중
> 복구용 파일 저장 · 제안 기능
  저장 위치 변경 / 다시 시도
내보낼 범위  메모리에 남은 공개 내용 + 선택한 초안
제외  비밀 값 · 버려진 출력 · 숨긴 reasoning
부분 자료이며 정확한 세션 재개 파일은 아닙니다.
''', '복구 대상/제외 범위를 먼저 표시한다. 실제 파일 쓰기 확인 후에만 저장 완료를 알린다.')
case('34-export-error', '내보내기', '파일 저장 실패', '복사 요청: 수신 확인 불가', ['Esc 결과로 복귀'], 'T28 T53 T56 T57 T64', 'P04 C11', '''
# 공개 결과와 보관된 출력
! /readonly/parser-report.md에 저장할 수 없습니다.
> 다른 경로로 저장
  같은 경로 다시 시도
클립보드 요청은 보냈지만 터미널 수신은 확인할 수 없습니다.
SSH/tmux 환경에서는 파일 저장을 사용할 수 있습니다.
현재 /copy는 최근 완료 assistant source 범위입니다.
이 화면의 범위 선택 내보내기는 제안입니다.
''', '경로 변경 뒤 저장 성공/실패를 확인한다. 복사 요청 전송을 복사 성공으로 표현하지 않는다.')
case('35-storage', '기록 보관', '예시: 820 MiB 사용', '선택: 오래된 작업 보관', ['Enter 대상 검토', 'Esc 돌아가기'], 'T36 T39 T64', 'P07 C11', '''
# 공간 정리 · 제안 기능
현재 작업과 연결된 분기는 자동 삭제하지 않습니다.
> 90일 이전 작업 보관 · 12개
  선택 기록 삭제 · 별도 확인
  보관 위치 열기
작업 수 / 실제 크기 / 연결 관계를 먼저 계산합니다.
실패 시 원본을 유지합니다.
''', '보관 성공 확인 후 원본 처리 정책을 적용한다. 삭제는 명시한 대상/범위 확인 후 수행한다.')
case('36-reconnect', '연결 복구', '전달 결과 불명', '입력 보관 · 제출 잠금', ['Esc 관찰 중단'], 'T20 T39 T42 T45 T64', 'C12', '''
! 요청을 보낸 뒤 연결이 끊겼습니다.
요청 ID  request-demo-44
실행 여부  확인 중
> 연결 상태 다시 확인
일반 초안  성능 영향도 알려줘
마지막 공개 화면은 오래된 상태입니다.
같은 명령을 자동으로 다시 보내지 않습니다.
재연결 후 요청 영수증과 현재 상태를 조회합니다.
''', '관찰 중단은 서버 작업 취소가 아니다. 재연결/실패/receipt 결과를 분리하고 확인 전 재전송을 막는다.')
case('37-help', '현재 화면 도움말', 'Chat · 기본 키 프로토콜', '선택: 입력과 취소', ['Esc 도움말 닫기'], 'T03 T14 T15 T38 T43 T54 T60', 'P01 C02', '''
# 지금 가능한 행동
Enter  일반 입력 보내기
Ctrl+G  외부 편집기로 여러 줄 작성
Esc / Ctrl+C  실행 중 턴 중단
# 지원 확인 필요
Shift+Enter  enhanced 키 지원 확인 후만 개행 안내
Alt 키/F-key가 통과하지 않으면 명령 목록 사용
# 명령 찾기
/help /model /status /find /resume /tree /fork
/copy /output /attach /prompt /compact /secrets
/new /interview /exit
/preview는 개발용 preview 실행에서만 사용합니다.
''', '실제 capability와 활성 상태에서 키 안내를 생성해야 한다. 이 fixture의 모든 키가 현재 Yo에 연결된 것은 아니다.')
case('38-compaction', '대화 정리', '실행 중 · 수동 요청', '요약 완료 전 사용량 미정', ['Esc 정리 중단'], 'T24 T42 T62 T22.b', 'P03 P12 C03', '''
# 대화 내용을 요약하고 있습니다
이유  사용자가 /compact 요청
기존 기록은 요약 성공 전 유지합니다.
실행 중에는 같은 요청을 중복 수락하지 않습니다.
입력한 다음 메시지는 대기 상태로 표시합니다.
현재 context  알 수 없음
완료 뒤 summary와 실제 관측값을 표시합니다.
''', '완료/실패/취소를 구분한다. 취소 후 메모리/초안을 버리지 않는다. 정확한 지원 여부는 구현 때 계약으로 확인한다.')
case('39-compaction-failed', '대화 정리', '실패 · 기존 기록 유지', '초안과 대기열 보관', ['Esc 대화로 복귀'], 'T24 T39 T62', 'P03 P12', '''
! 모델 응답을 받지 못했습니다.
시도  2/3 · 다음 재시도까지 5초
자동 재시도 취소 / 나중에 다시 정리
기존 대화로 계속 작업할 수 있습니다.
[완료 상태의 표시 예]
summary 저장 완료 · 정리 전 24,000 tokens
정리 후 사용량은 관측 전까지 알 수 없음
''', '재시도 시간/횟수는 실제 상태에서만 표시한다. 완료 예시는 별도 상태로 구현한다.')
case('40-plan', '작업 계획', 'Proposed plan · 미확정', '선택: 완료 기준 검토', ['Esc 대화로 복귀'], 'T22.a T25.a T59', 'P11 C06', '''
# 빈 입력 처리
[완료] 코드와 기존 계약 읽기
[진행] 완료 기준 확인
[대기] 구현과 회귀 검증
완료 기준  빈 입력은 빈 결과 / 기존 API 유지
계획 수정 / 계획 확정
native Plan은 자체 Session 기록을 소유합니다.
Goal 예산이나 child task 생성과는 별도 단계입니다.
''', '초안 계획과 확정된 계획을 구분한다. 최종 문서가 도착하면 같은 source identity로 갱신한다.')
case('41-tasks', '병렬 작업', '2 실행 · 1 승인 대기', '부모 작업: parser 개선', ['Esc 대화로 복귀'], 'T22 T42 T58 T59', 'C03 C05', '''
# 작업별 상태 · 제안 기능
[실행] A · 회귀 테스트 검토 · 읽기 전용
[실행] B · 문서 검토 · 읽기 전용
[승인 대기] C · process 실행
C의 승인 대기는 독립 읽기 A/B를 멈추지 않습니다.
공유 쓰기 충돌은 host에서 직렬화합니다.
Goal 예산  12k / 30k tokens · 예시
중단 대상  선택 작업 / 전체 Goal을 구별
''', '상태는 host 결과로 갱신한다. 화면만으로 병렬 승인/실행이 구현됐다고 간주하지 않는다.')
case('42-memory', '프로젝트 메모', '수동 관리 · 제안', '선택: parser API 규칙', ['Enter 원문 보기', 'Esc 돌아가기'], 'T25.a T28 T59', 'P11 C11', '''
> parser API 규칙
  범위: project · 출처: docs/parser.md
  사용자 확인: 2026-10-04 · 예시
원문과 적용 범위를 확인한 뒤 편집합니다.
검색 결과는 보조 자료이며 기록의 authority가 아닙니다.
비밀 값과 일시적인 테스트 실패는 자동 저장하지 않습니다.
''', '명시한 파일/범위만 변경한다. 숨은 자동 학습 기능을 전제하지 않는다.')
case('43-mcp', '도구 연결', '호스트: native', 'example-docs · 미지원', ['Esc 돌아가기'], 'T22 T39 T59 T22.c', 'P09 C08', '''
# 연결 상태 · 제안 기능
example-docs  연결되지 않음
원인  이 프로필은 필요한 resource 기능을 지원하지 않음
요구 기능  text tools + resource read
지원 기능  실험적 text tools만
delegated 호스트의 MCP 표시와 native 지원은 다릅니다.
설정 보기 / 지원 범위 확인
''', '실제 서버 호환 검증 전 사용 가능 표시를 하지 않는다. 설정 실패는 기존 세션을 유지한다.')
case('44-terminal', '터미널 표시', 'SSH / tmux · 예시 환경', '텍스트 모드', ['Esc 돌아가기'], 'T03 T47 T50 T51 T52 T53 T54 T55 T56 T57', 'P01 P02 C02 C09', '''
테마  터미널 기본 · 색 외에 + / - / ! 표시
움직임 줄이기  켜짐 · 정적인 진행 텍스트
이미지 프로토콜  확인 안 됨 · 파일 정보 표시
키 확장  확인 안 됨 · 외부 편집기 안내
마우스  스크롤 지원 여부를 확인해야 함
한국어 가나다 / é 결합 문자
기존 scrollback은 resize로 다시 쓰지 않습니다.
화면 복귀 후 초안과 원래 선택을 복원합니다.
''', '실제 probe/설정 값으로만 능력을 표시한다. 이 fixture는 SSH/tmux/Unicode17 동작 검증이 아니다.')
case('45-documents', '문서', '원문 유지 · 좁은 폭 재배치', '선택: 결과 표', ['Esc 대화로 복귀'], 'T25 T26 T27 T59 T25.a', 'P11 C09', '''
# 회귀 검증 결과
| 입력 | 결과 |
| 빈 문자열 | 빈 목록 |
| 한글 | 토큰 2개 |
# 좁은 화면용 같은 값
입력: 빈 문자열 / 결과: 빈 목록
입력: 한글 / 결과: 토큰 2개
도식: input -> parser -> tokens
렌더링 미지원 시 Mermaid 원문 보기
status/help 문서도 같은 제목/목록 계층을 사용합니다.
''', '표의 열 값/순서를 보존한다. 렌더러 실패 시 원문을 열 수 있어야 한다.')
case('46-resources', '도구 자료', '일부 결과 · 원문 보존', '선택: 문서 resource', ['Enter 보관 자료 보기', 'Esc 대화로 복귀'], 'T22 T22.c T22.d T22.e T22.f T22.g T22.h T59', 'P10 C10', '''
# Resource link
이름  parser specification
URI  docs://project/parser/spec
크기  알 수 없음 · 자동 가져오기 안 함
# Embedded 자료
text/plain · UTF-8 · 1,248 bytes
binary  원문 표시 불가 · metadata 보존
# 파일 묶음 읽기
src/parser.rs:1-80 · 성공 · 다음 범위 있음
tests/missing.rs · 실패: 파일 없음
# 디렉터리
5개 표시 · 전체 개수 알 수 없음
# 검색
empty · 최대 20개 일치 · 일부 결과
# 실행
host local / call7 / exit 1 / retained 8 KiB
''', '각 자료 유형의 원래 필드와 source를 보존한다. unknown을 0이나 전체 결과로 바꾸지 않는다.')
case('47-media', '첨부와 미디어', '텍스트 대체 표시', '이미지 원본 별도', ['Esc 대화로 복귀'], 'T04 T10 T29 T56 T29.a', 'P10 C01 C10', '''
# screenshot.png
이미지  1200 x 800 · 본문 표시 미지원
설명  parser 오류 메시지 화면 · 예시
원본 경로 보기 / 파일로 저장
# audio.wav
오디오 재생 미지원 · metadata만 표시
# 되돌리기로 복구된 [image] 문자열
! 첨부 데이터 없음 · 다시 첨부해야 전송할 수 있습니다.
문자 marker를 실제 이미지처럼 표시하지 않습니다.
''', '표시 여부와 전송 payload를 별개로 관리한다. 지원하지 않는 미디어를 자동 실행하지 않는다.')
case('48-interview', '질문 작업 사본', '기존 사본 관리', '선택: 계속하기', ['Enter 계속하기', 'Esc 일반 초안으로'], 'T18 T20 T63', 'C06', '''
# /interview
진행 중 사본  입력 정책 확인 · 2개 질문
> 계속하기
  사본 닫기
종료된 사본  읽기 / 폐기
새 인터뷰를 만드는 명령은 아닙니다.
일반 초안  성능 영향도 알려줘 · 보관 중
''', '기존 사본의 lifecycle만 조작한다. 닫기/폐기 뒤 일반 초안 소유권을 복원한다.')
case('49-preview', '개발용 Preview', '제품 세션과 분리', 'fixture: long-output', ['/exit 원래 화면'], 'T47 T60 T61', 'C02 C09', '''
# 개발 전용 렌더링 예시
일반 live 실행에는 이 명령이 노출되지 않습니다.
이 화면의 도구 결과는 실제 실행 기록이 아닙니다.
/preview로 다른 fixture 선택
/exit로 진입 전 화면과 초안 복원
Esc가 항상 preview 종료라는 가정을 하지 않습니다.
''', 'preview 상태를 제품 세션에 쓰지 않고 진입 전 상태를 복원한다.')
case('50-editor-failed', 'Chat', '외부 편집 결과 미적용', '기존 초안 보존', ['Ctrl+G 다시 편집'], 'T04 T11 T55', 'P01 C01', '''
! 편집 중 참조 범위가 모호하게 변경됐습니다.
기존 초안  성능 영향도 알려줘
참조  @src/parser.rs · snapshot 보존
편집한 텍스트는 자동 제출하지 않았습니다.
참조를 유지하도록 수정하거나 일반 텍스트로 다시 작성하세요.
터미널 복귀 오류는 별도 진단으로 표시합니다.
''', '외부 편집 실패가 기존 초안/첨부를 덮지 않는다. raw mode/cursor 복원은 실제 PTY 테스트 필요.')
case('51-picker-stale', '모델 선택', '목록 갱신 중', '선택 수락 잠금', ['Esc 돌아가기'], 'T35 T39 T40 T45', 'P05 C08', '''
검색  vision
! 이전에 고른 모델을 현재 목록에서 확인할 수 없습니다.
목록을 확인하는 동안 Enter는 적용하지 않습니다.
빈 결과라면 검색 지우기 / 계정 연결
모두 비활성이라면 각 항목의 이유 보기
새 목록의 첫 항목을 자동 수락하지 않습니다.
''', '공개한 선택과 현재 항목 ID가 일치한 뒤만 수락한다. 갱신 실패는 원인을 표시한다.')
case('52-scrollback', 'Chat', '이전 내용 읽는 중', '새 출력 18줄', ['Esc 중단', 'Ctrl+C 중단'], 'T21 T44 T46 T47 T48 T49 T58', 'P01 P10 C03 C10', '''
# 이전 도구 #call3
[접힘] src/parser.rs 읽기 · 80줄
선택 항목 펼치기와 전체 펼치기는 별도 행동입니다.
현재 읽는 위치  call3 / source line 24
새 출력으로 읽던 위치를 끌어내리지 않습니다.
최신으로 이동 · 명시적 행동
폭 80 -> 24 -> 80에도 같은 source 위치를 유지합니다.
inline에 이미 발행된 history는 재작성하지 않습니다.
''', 'source anchor를 보존하는 설계 제안. fixture 폭 변환은 실제 스크롤/resize 실행 검증이 아니다.')
case('53-reconnect-restored', '연결 복구', '복구됨 · 중복 전송 없음', '요청 #44 결과 확인', ['Esc 대화로 복귀'], 'T20 T39 T42 T45', 'C12', '''
[완료] 요청 영수증과 현재 상태를 조회했습니다.
요청 #44  호스트 수락됨 · 도구 실행 완료
동일 요청을 다시 보내지 않았습니다.
일반 초안  성능 영향도 알려줘 · 복원
오래된 승인 화면의 응답은 폐기했습니다.
입력 잠금 해제는 새 공개 frame 확인 뒤 수행합니다.
''', '조회 결과가 없음/불명이라면 계속 불명 상태를 유지한다. 이 성공 예시는 실행 증거가 아니다.')
case('54-empty-list', '이전 작업', '이 프로젝트 · 0개', '선택 가능한 작업 없음', ['Esc 돌아가기'], 'T36 T39 T40', 'P07 C07', '''
# 검색에 맞는 작업이 없습니다
검색어  parser
검색 지우기 / 전체 프로젝트에서 찾기
Enter로 빈 항목을 수락하지 않습니다.
현재 초안은 그대로 보관 중입니다.
''', '범위를 넓힌 뒤에도 없는 경우 빈 이유를 유지한다. 로딩과 빈 결과는 별도 상태.')
case('55-tiny', '승인', 'Proposed', '승인 잠금', ['Esc 중단'], 'T14 T16 T43 T44 T45', 'C05', '''
창을 늘리세요.
선택과 실행 내용을 함께 표시할 공간이 없습니다.
Enter로 승인하지 않습니다.
''', '24x8 예시: 크기 부족이면 수락을 잠그고 명시한 중단 행동만 유지한다.')

# The real composer stays anchored above actions; it is not transcript content.
for item in CASES:
    if item['id'] in {'07-chat', '19-draft-restored'}:
        start = item['body'].index('# 일반 초안')
        end = start + 1
        composer = []
        while end < len(item['body']) and item['body'][end].startswith('> '):
            composer.append(item['body'][end])
            end += 1
        item['composer'] = composer
        del item['body'][start:end]
    if item['id'] == '08-working-queue':
        item['composer'] = ['> 성능 영향도 알려줘']
        item['focus'] = '일반 초안 · 큐 2개'
        item['body'].remove('[일반 초안 보관] 성능 영향도 알려줘')

case('56-palette', '명령 찾기', '입력: /mo · 1개', '선택: /model', ['Enter 명령 선택', 'Esc 목록 닫기'], 'T15 T38 T40 T60', 'P01 C01 C02', '''
> /model  현재 또는 다음 턴 모델 선택
/help     현재 화면의 행동과 명령
/status   호스트·모델·사용량 확인
검색어를 바꾸면 표시된 결과에서 다시 선택합니다.
선택한 명령과 실제 dispatch 대상이 같아야 합니다.
''', '선택한 명령을 활성 화면에 맞게 dispatch한다. 목록 갱신 중 이전 행 번호로 실행하지 않는다.')
case('57-actions', '현재 화면 행동', '질문 #r8 · 제안 메뉴', '선택: 이전 질문', ['Enter 선택 실행', 'Esc 메뉴 닫기'], 'T18 T19 T38 T43 T54', 'P09 C02 C06', '''
# Ctrl+K로 여는 문맥 행동 메뉴
> 이전 질문
  답변 검토
  메모 편집
  도움말
일반 초안과 질문 답변은 각각 보관 중입니다.
활성 요청의 capability에 없는 행동은 숨기거나 이유를 표시합니다.
비밀 입력에서는 전체 지우기, 승인에서는 상세 보기를 제공합니다.
''', '선택된 행동 ID를 실행하고 이전 focus로 복귀한다. Ctrl+K는 제안 binding이며 기존 kill-line과 충돌 조정이 필요하다.')
case('58-editing', '입력 편집', '선택 영역 · 초안만 수정', '일반 초안 · 미전송', ['Esc 선택 해제', 'Ctrl+G 외부 편집'], 'T02 T03 T04 T51', 'P01 C01', '''
# 선택/커서 표시의 텍스트 대체
가나다 [선택: 빈 입력] café
                     ^ 커서
선택 범위  입력의 grapheme 4..8 · 예시
Undo는 직전 편집 단위를 복원합니다.
참조/첨부 marker의 데이터 수명은 별도 확인합니다.
한글 두 셀과 é 결합 문자를 서로 다르게 처리합니다.
''', '이 도식은 실제 selection/cursor 시뮬레이션이 아니다. caret와 selection 렌더링 및 IME는 실제 terminal test 대상.')
case('59-changes-reported', 'Changes', 'Reported · 보고 자료', '쓰기 전후 검증 없음', ['Esc 대화로 복귀'], 'T30 T31 T46', 'C09', '''
# 외부 호스트가 보고한 diff
src/parser.rs
-    return Err(EmptyInput);
+    return Ok(Vec::new());
원문은 보존했지만 실제 쓰기 전후 비교 근거가 없습니다.
Recorded 완료 편집과 같은 증거로 취급하지 않습니다.
현재 Git diff는 별도 조회입니다.
''', '호스트 보고 자료를 provenance와 함께 유지한다. 검증된 before/written 증거 없이 Recorded로 승격하지 않는다.')
case('60-compaction-retry', '대화 정리', '재시도 대기 · 2/3', '5초 후 재시도 · 예시', ['Esc 재시도 취소'], 'T24 T39 T42 T62', 'P03 P12', '''
! 이전 요청에서 모델 응답을 받지 못했습니다.
다음 시도 전 기존 기록과 초안을 유지합니다.
입력한 다음 메시지는 일시 정지 상태입니다.
재시도 취소는 원래 대화 삭제와 다릅니다.
시간과 횟수는 실제 host 상태를 표시해야 합니다.
''', '타이머가 끝나면 새 정리 요청을 시작한다. 취소 시 pending retry만 해제한다.')
case('61-compaction-complete', '대화 정리', '완료 · summary 저장됨', 'context 사용량 미관측', ['Esc 대화로 복귀'], 'T24 T62 T22.b', 'P12', '''
[완료] 대화 요약을 저장했습니다.
정리 전  24,000 tokens · 예시
정리 후  아직 알 수 없음
요약 내용 펼치기 / 원래 공개 기록 보기
대기열은 각 메시지의 종류와 첨부를 유지합니다.
''', '새 context 관측값이 생긴 뒤만 사용량을 갱신한다. summary 완료와 다음 메시지 실행을 구분한다.')
case('62-answer-review', '답변 검토', '2/2 답변 작성됨', '선택: 전체 답변 제출', ['Enter 전체 제출', 'Esc 질문으로'], 'T18 T20 T43', 'C06', '''
# 요청 #r8
1. 빈 입력 결과  빈 결과 반환
   메모  현재 API 유지
2. 회귀 테스트  추가
> 전체 답변 제출
질문 수정은 Ctrl+K 행동 메뉴에서 선택합니다.
일반 초안  성능 영향도 알려줘 · 전송 대상 아님
''', '유효한 request ID에 전체 답변을 한 번 제출한다. 성공/거절/만료 뒤 일반 초안 복원을 분리한다.')
case('63-approval-expired', '승인 요청', '만료 · 제출 잠금', '요청 #r7 종료됨', ['Esc 대화로 복귀'], 'T16 T20 T39 T45', 'C05 C12', '''
! 상세 내용을 읽는 동안 요청이 종료됐습니다.
Enter는 이전 선택을 제출하지 않습니다.
마지막 선택  이번 요청 거절
새 요청  #r9 · 별도 대기
이전 승인 선택을 새 요청에 자동 적용하지 않습니다.
''', '폐기된 frame과 request의 선택은 수락하지 않는다. 새 요청은 별도 focus와 기본값으로 표시한다.')
case('64-connect-error', '모델 연결', '실패 · 인증 코드 만료', '선택: 새 연결 시작', ['Enter 다시 연결', 'Esc 시작 화면'], 'T39 T42 T64', 'P03 C08', '''
! 브라우저 인증 시간이 만료됐습니다.
계정 정보는 변경되지 않았습니다.
> 새 코드로 연결 시작
사용하지 않은 이전 코드는 폐기합니다.
작성 중인 일반 초안은 그대로 보관합니다.
''', '새 요청 ID로 인증을 시작한다. 이전 인증 완료 이벤트를 새 연결에 적용하지 않는다.')

by_id = {c['id']: c for c in CASES}
# Explicit operation bindings. All additions here are proposals, not existing Yo keys.
for id, action in {
    '06-settings-failed':'Enter 저장 재시도', '14-approval':'Ctrl+V 상세',
    '20-secret':'Ctrl+U 전체 지우기', '30-session-error':'Enter 다시 읽기',
    '33-rescue':'Enter 저장 위치', '34-export-error':'Enter 다른 경로',
    '36-reconnect':'Enter 상태 확인', '39-compaction-failed':'Enter 다시 정리',
}.items():
    by_id[id]['actions'].insert(0, action)
by_id['14-approval']['body'].insert(0, '방향키로 선택 이동 · 기본 선택은 거절')
by_id['14-approval']['after'] += ' Ctrl+V 상세 후 Esc로 돌아온다. 방향키로 선택을 바꾸면 Enter 의미도 함께 바뀐다.'
by_id['20-secret']['after'] += ' Ctrl+U는 기존 kill-line과 다른 전체 지우기로 바꾸는 제안이다.'
by_id['17-question']['focus'] = '빈 입력의 결과는?'
by_id['17-question']['composer'] = ['> 빈 결과 반환', '메모: 현재 API 유지', '일반 초안 보관']
by_id['17-question']['body'] = ['요청 #r8 · 질문 전용 입력', '방향키 선택 / Ctrl+K 행동',
    '다른 선택: 오류 반환', '일반 초안: 성능 영향도 알려줘', '참조 1개는 일반 초안에 보관',
    '현재 메모를 편집하려면 Ctrl+K 행동을 엽니다.']
by_id['18-question-choice']['composer'] = ['회귀 테스트 추가?', '일반 초안 보관']
by_id['16-approval-detail']['body'] = by_id['16-approval-detail']['body'][:10] + [
    '파일 변경 제안은 이 실행 요청에 포함되지 않습니다.',
    'src/parser.rs 편집 제안은 별도 승인 요청 #r8에서 확인합니다.',
    '상세 화면에서는 승인하지 않습니다.']
by_id['24-changes-recorded']['body'][0] = '# 완료 편집 #call9 · before/written 비교'
by_id['24-changes-recorded']['body'].insert(1,'근거  쓰기 직전 snapshot과 완료된 written snapshot')
by_id['39-compaction-failed']['body'] = ['! 모델 응답을 받지 못했습니다.',
    '자동 재시도는 종료됐습니다.', '> 정리 다시 요청', '기존 기록·초안·대기열을 유지합니다.',
    '대화로 돌아가 기존 context로 계속 작업할 수 있습니다.']
by_id['39-compaction-failed']['focus'] = '선택: 정리 다시 요청'
by_id['39-compaction-failed']['after'] = 'Enter로 명시적 재요청. Esc는 기존 대화로 복귀. 자동 재시도/완료는 60/61번 별도 장면.'
# A context menu gives secondary actions an explicit, inspectable route.
menus = {
 '03-model':['검색 편집','새 세션 기본값 설정'], '04-model-failed':['이미지 지원 모델 선택','첨부 제거'],
 '05-settings':['현재 값 변경','새 세션 기본값 저장'], '08-working-queue':['대기 메시지 열기','초안 편집'],
 '09-queue-paused':['선택 메시지 편집','선택 메시지 삭제','대기열 재개','새 대화 시작'],
 '13-find':['검색어 편집','검색 범위 선택'], '17-question':['이전 질문','답변 검토','메모 편집'],
 '18-question-choice':['이전 질문','답변 검토'], '20-secret':['전체 지우기'],
 '22-secrets':['복구 값 삭제'], '25-changes-current':['다시 조회'],
 '26-output':['보관 원문 찾기','보관 원문 복사'], '27-source':['선택 코드 복사'],
 '29-sessions':['검색 편집','다음 페이지','전체 프로젝트','미리보기','이름 변경'],
 '33-rescue':['저장 위치 변경','저장 다시 시도'], '34-export-error':['다른 경로로 저장','같은 경로 다시 시도'],
 '35-storage':['대상 검토','삭제 대상 선택'], '41-tasks':['선택 작업 중단','전체 Goal 중단'],
 '43-mcp':['설정 보기','지원 범위 확인'], '45-documents':['원문 보기'],
 '47-media':['원본 경로 보기','파일 저장','이미지 다시 첨부'],
 '48-interview':['사본 계속','사본 닫기','종료 사본 읽기','종료 사본 폐기'],
 '51-picker-stale':['검색 지우기','계정 연결','비활성 이유 보기'],
 '52-scrollback':['선택 항목 펼치기','전체 펼치기','최신으로 이동'],
 '54-empty-list':['검색 지우기','전체 프로젝트'], '61-compaction-complete':['요약 펼치기','공개 기록 보기'],
 '62-answer-review':['질문 수정'],
}
for id, entries in menus.items():
    by_id[id]['menu'] = entries
    if id != '20-secret': by_id[id]['actions'].append('Ctrl+K 행동')

by_id['10-queue-edit']['composer'] = ['> 변경 근거와 테스트 결과를 정리해줘']
by_id['10-queue-edit']['body'].remove('> 변경 근거와 테스트 결과를 정리해줘')
by_id['56-palette']['body'] = ['> /model  현재 또는 다음 턴 모델 선택',
    '검색어를 바꾸면 표시된 결과에서 다시 선택합니다.',
    '선택한 명령과 실제 dispatch 대상이 같아야 합니다.']
by_id['45-documents']['body'] = ['# 회귀 검증 결과', '| 입력 | 결과 |',
    '| 빈 문자열 | 빈 목록 |', '| 한글 | 토큰 2개 |', '# 처리 흐름',
    'input -> parser -> tokens', '도식 미지원 시 Mermaid 원문 보기',
    '표의 같은 값을 좁은 화면에서는 세로 목록으로 표시합니다.']
by_id['45-documents']['body_narrow'] = ['# 회귀 검증 결과', '입력: 빈 문자열', '결과: 빈 목록',
    '', '입력: 한글', '결과: 토큰 2개', '# 처리 흐름', 'input -> parser', 'parser -> tokens',
    '도식 미지원: 원문 보기', 'Ctrl+K 행동에서 원문을 선택합니다.']

case('65-long-command', '승인 상세', 'Proposed · 미실행', '복귀 선택: 이번 요청 거절', ['Esc 승인으로 복귀'], 'T16 T17 T43 T46', 'C05', '''
# 긴 명령 · 전체 argv
호스트 local / cwd /work/parser
실행 파일 /usr/bin/cargo
argv[0] test
argv[1] --manifest-path
argv[2] /work/parser/crates/parser/Cargo.toml
argv[3] --package
argv[4] parser
argv[5] --test
argv[6] empty_input
argv[7] --
argv[8] --exact
argv[9] parse_empty_input
argv[10] --nocapture
프로필 workspace-test · target 쓰기만 허용
네트워크 차단 · source 파일 쓰기 없음
마지막 인수까지 이 화면에서 읽을 수 있습니다.
내용 페이지 이동은 승인 선택을 바꾸지 않습니다.
''', '상세 내용을 모두 읽고 Esc로 원래 승인 선택으로 돌아간다. 긴 argv를 줄임표로 제거하지 않는다.')
case('66-streaming', 'Chat', '응답 받는 중', '미완료 답변 · 00:06', ['Esc 중단', 'Ctrl+C 중단'], 'T21 T24 T25 T26 T42', 'P11 C03', '''
# yo
빈 입력은 기존 반환형을 유지하면서 먼저 처리하겠습니다.
# 아직 닫히지 않은 코드 블록
```rust
fn parse(input: &str) ->
[응답 계속 받는 중]
이 문장은 완료된 답변으로 복사/저장하지 않습니다.
공개 reasoning은 숨겨져 있습니다. 답변 본문은 계속 보입니다.
''', '미완료 Markdown을 안전한 텍스트로 보여주고 완료 시 다시 배치한다. 중단 뒤 부분 답변/중단 상태를 보존한다.')
