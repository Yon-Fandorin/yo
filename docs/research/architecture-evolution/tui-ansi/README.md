# Yo TUI ANSI 디자인 예시

> 상태: 비권위적 설계 자료. **현재 Yo의 동작 화면이 아닌 구현용 제안**이다.
> 2026-10-05 · 실제 Pi/Codex 소스와 snapshot을 읽어 재구성했다.

[오프라인 갤러리](index.html)에서 **66개 장면**을 크기·색상·내용 페이지별로 비교할 수 있다.
실제 ESC 코드가 들어간 ANSI **397개**, 같은 셀을 색 없이 읽는 TXT **265개**를 저장했다.
100×34, 80×30, 40×24, 24×18을 제공하고, 승인 수락이 잠기는 24×8 예시도 포함한다.
80열은 dark/light/mono 세 변형, 다른 크기는 dark 변형이다.

- [전체 장면·64개 요소와 10개 하위 요소 목차](coverage.md)
- [Pi·Codex의 24개 소스 anchor와 적용 판단](sources.md)
- [장면 정의와 이후 동작](cases.py), [생성기](generate.py), [파일·크기·해시 manifest](manifest.json)
- [독립 검수·수정·실행한 검사](review.md)

## 바로 보기

HTML은 파일 탐색기나 브라우저에서 `index.html`을 연다. 외부 폰트/CDN/네트워크 요청은 없다.
화면 안의 키/행동은 제품 설계이고, 갤러리에서 작동하는 것은 장면·크기·색·페이지 전환뿐이다.

프로젝트 루트에서 실행한다.

```sh
# 실제 ANSI 한 페이지만 터미널에 출력
python docs/research/architecture-evolution/tui-ansi/generate.py --show 08-working-queue --width 80

# 24열 질문: 입력·선택·일반 초안 보관이 페이지마다 유지되는지 보기
python docs/research/architecture-evolution/tui-ansi/generate.py --show 17-question --width 24 --page 2

# 최소 높이에서 숨은 선택을 승인하지 않는 상태
python docs/research/architecture-evolution/tui-ansi/generate.py --show 55-tiny --width 24 --height 8

# 밝은 배경과 단색
python docs/research/architecture-evolution/tui-ansi/generate.py --show 23-changes-proposed --theme light
python docs/research/architecture-evolution/tui-ansi/generate.py --show 20-secret --theme mono

# 재생성 / 구조·의미 불변조건·파일 drift 검사
python docs/research/architecture-evolution/tui-ansi/generate.py
python docs/research/architecture-evolution/tui-ansi/generate.py --check
```

ANSI 파일 전체를 `cat`해도 된다. 다중 페이지 파일은 고정 높이 화면들을 세로로 이어 놓았으므로
한 화면씩 보려면 `--show … --page N`을 사용한다. 화면 지우기·커서 이동·alternate screen·OSC52
클립보드 제어는 넣지 않았다. 색상/굵기 SGR과 줄바꿈만 사용한다.
`less -R`로 열면 **pager 자체의 키**로 읽으며, 화면에 인쇄된 제품 키는 작동하지 않는다.

대표 파일:

| 검토 과제 | ANSI | 일반 텍스트 |
|---|---|---|
| 실행 + 대기열 + 중단 안내 | [80열](ansi/08-working-queue.80x30.dark.ansi) | [24열](plain/08-working-queue.24x18.txt) |
| 질문 전용 입력과 일반 초안 보관 | [80열](ansi/17-question.80x30.dark.ansi) | [24열](plain/17-question.24x18.txt) |
| 비밀 입력 전체 지우기 | [입력됨](ansi/20-secret.80x30.dark.ansi) | [전체 지운 뒤](plain/21-secret-cleared.24x18.txt) |
| 승인 상세 → 원래 선택 | [승인](ansi/14-approval.80x30.dark.ansi) | [상세](plain/16-approval-detail.24x18.txt) |
| 변경 근거 구별 | [Proposed](ansi/23-changes-proposed.80x30.dark.ansi) | [Recorded](plain/24-changes-recorded.24x18.txt) · [Reported](plain/59-changes-reported.24x18.txt) · [Current](plain/25-changes-current.24x18.txt) |
| 기록 공백·복구 범위 | [저장 실패](ansi/33-rescue.80x30.dark.ansi) | [내보내기 실패](plain/34-export-error.24x18.txt) |

## 디자인 규칙

1. **본문과 행동의 위계.** 제목 → 현재 상태 → 읽는 내용 → 고정된 선택/입력 → 현재 행동 순서다.
   입력 초안과 질문/선택은 내용 페이지를 넘겨도 하단에 남는다. 보조 설명은 페이지로 나누고 버리지 않는다.
2. **가장 먼저 보존할 정보.** 실행 중 Esc/Ctrl+C, 질문의 선택과 일반 초안 보관, 비밀 입력 유무,
   Proposed/Recorded/Reported/Current, 불명·부분·미저장을 경로/사용량 장식보다 우선한다.
3. **선택과 의미를 함께.** `>`와 구체적인 이름으로 선택을 표시한다. Enter 안내는 실제 선택 행동에서 도출해야 한다.
   너무 작은 높이에서는 선택을 감추고 수락시키지 않으며 명시적으로 잠근다.
4. **원문과 표시를 분리.** 표시용 줄바꿈/색/행 번호는 복사 원문에 포함하지 않는다. retained 출력은 전체 출력이 아니다.
   표와 media의 fallback은 원래 값·source 접근을 보존한다.
5. **터미널 기본 배경 존중.** ANSI는 배경색을 강제하지 않는다. foreground 6개 역할(text/muted/accent/success/warn/error)과
   굵기로 위계를 만든다. 갤러리의 배경은 비교용이다. 색만으로 성공/실패/선택을 구별하지 않는다.
6. **실패 뒤 행동.** 실패 원인·보존된 자료·다시 시도/복귀를 함께 보여준다. 전달 불명은 자동 재전송하지 않는다.

Pi에서는 입력창 주변 상태·간결한 footer·모델/세션 목록을, Codex에서는 상황별 행동 안내·대기 메시지 미리보기·
승인 상세·질문 전용 입력을 참고했다. Yo의 typed content, source 보존, 요청/기록 의미를 함께 유지한다.
Pi의 큐 복원 시 텍스트 합치기와 예제 extension의 단순 위험 명령 승인은 도입 근거로 사용하지 않았다.

## 키와 현재 기능의 경계

**전체 갤러리는 제안이다.** 아래 키를 현재 Yo 사용 설명서로 사용하면 안 된다.
기존 기능에 바탕을 둔 장면도 정보 배치·상태·선택 동작은 달라질 수 있다.

| 항목 | 현재 코드에서 확인한 것 | 이 자료가 제안하는 변경 |
|---|---|---|
| 일반 외부 편집 | Chat에서 Ctrl+G, pending request/overlay에서는 일반 편집 경로 비활성 | 같은 경계 유지, 오류/초안 보존을 더 잘 표시 |
| 승인 | 요청 capability에 따라 decline 또는 Interrupt, 일부 footer 의미 불일치 | 표시한 선택에서 Enter 의미 생성, Ctrl+V 상세와 Esc 왕복 |
| 질문 | 현재 일반 입력 editor와 요청 routing 공유 문제 | 전용 답변 composer·일반 초안 보관·Previous/검토/최종 제출 |
| 비밀 Ctrl+U | 현재 줄 앞부분 kill-line | **비밀 입력에서만 전체 임시 값 지우기**, 입력됨/미입력으로 길이 비공개 |
| Ctrl+K 행동 | 통합 문맥 행동 메뉴를 구현한 것으로 확인하지 않음 | 장면별 `menu` 목록을 표시하는 제안. 기존 편집 binding 충돌/대체 키를 구현 때 확정 |
| PgUp/Dn | 기존 각 view의 paging/scroll owner가 있음 | 이 디자인의 내용 페이지 이동 제안. gallery 버튼이나 pager 키와 구분 |
| Enter/방향키/Esc | 현재 view/request마다 routing이 다름 | 화면에 표시한 선택/수락/취소 의미. 미지원 capability에서는 잠금/이유 표시 |
| Shift+Enter | 기본 binding은 있으나 키 확장 협상/SSH 통과 검증 부족 | 기본 프로토콜에서는 무조건 안내하지 않고 확인된 외부 편집 경로 제공 |
| 통합 설정/검색/큐 편집 | 개별 기능 또는 제한된 경로 존재 | searchable model/session, 항목별 큐 편집·삭제·재개, 새 기본값과 현재 값 분리 |
| 복구 export/보관/Current Git diff | 제안한 통합 여정은 현재 동작으로 검증하지 않음 | 범위·부분 자료·실패를 표시한 별도 화면 |
| Plan/Goal/task/memory/native MCP/재접속 | 호스트별 구현·지원 차이가 있음 | 후속 설계의 사용자 화면. 승인 재사용·병렬화·완전 복구가 구현됐다는 뜻 아님 |
| `/preview`, `/interview`, F2/F3 | preview는 개발용, interview는 기존 사본 관리, F2 Transcript/F3 Request 진단 | 이 의미를 보존하고 일반 기능으로 과장하지 않음 |

추가 제안 행동은 [장면 정의](cases.py)의 `actions`, `menu`, `after`에 있다. 특히 Ctrl+K는 확정된 binding이 아니며,
플랫폼/편집기 충돌을 검토한 뒤 하나의 action registry에서 키 표시·dispatch·접근 가능한 메뉴를 생성해야 한다.

## 검증 범위와 다음 구현

생성기는 추가 패키지 없이 Python 표준 라이브러리만 사용한다. 한글 W/F=2, 결합문자=0, ambiguous=1인
fixture 문자 집합을 대상으로 한다. Yo의 실제 Unicode 17 grapheme/emoji 폭 엔진을 대체하지 않는다.
브라우저 폰트와 터미널 폰트의 한글/특수 문자 폭이 다를 수 있다.

64개 요소 매핑은 **화면 검토 범위**이며 모든 상태나 상호작용의 테스트 완료 수가 아니다.
특히 58번 selection/caret는 표기 예시이고 실제 편집기는 아니다. resize/frame 수락, 빠른 출력,
raw mode 복원, clipboard, screenreader, SSH/tmux, 실제 모델/도구 호출은 이 정지 화면으로 검증할 수 없다.
현재 연구 자료만 추가했으며 production TUI와 Methexis 계약은 바꾸지 않았다.

구현 시 관련 장면·현재 owner·accepted contract를 함께 선택한 뒤 실제 state/action 연결과 오류 여정을 검증한다.
기존 `rendering-parity`의 escaped ANSI golden(작은 Surface 테스트)과 이 실제 ESC 디자인 파일은 목적이 다르므로
그 golden을 교체하거나 이 제안을 그대로 통과 기준으로 삼지 않는다.
