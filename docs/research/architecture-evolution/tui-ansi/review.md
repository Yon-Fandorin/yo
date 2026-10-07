# ANSI 디자인 자료 최종 검수

2026-10-05 · 최종 후보: **66개 장면 / ANSI 397개 + TXT 265개 / ANSI 내용 페이지 522개**.
정적 설계 자료의 독립 재검수를 완료했다. 검토한 범위에서 미해결 주요 지적은 없다.
현재 제품 구현이나 터미널 실사용 품질을 승인한 결과는 아니다.

## 검수 분담과 결과

| 검수 | 범위 | 최종 결과 |
|---|---|---|
| Pi reference/content agent | Pi 실제 소스 12곳과 인접 테스트, 장면·내용·행동 연결, 24열 초안·질문·큐·표·긴 명령 | read-only 검수 및 수정 후 재검수 완료. 주요 미해결 지적 없음 |
| Codex independent agent · gpt-6-astra/high | 새 문맥의 독립 소스/설계 검토, Codex 소스 12곳과 snapshot, 실제 ANSI/TXT·generator·gallery·provenance | read-only 검수 및 최종 변경 재검수 PASS. 파일 해시와 libc 폭 검사도 독립 수행 |
| 주 작업자 | 최종 fixture 생성, 목차 역방향 대조, SGR/plain 동등성, JS·링크·팔레트 검사, 지적 반영 | 아래 검사 PASS |

외부 서비스에 검수 자료를 보내지 않았다. upstream 소스는 읽기만 했다.
검수자는 repository를 편집하지 않았고 지적을 주 작업자가 평가하여 반영했다.

## 반영한 주요 지적

- 일반 초안/질문 선택/대기 메시지 편집이 내용 페이지 밖으로 밀려난 채 Enter가 활성화되던 문제:
  실제 입력과 선택을 고정 영역으로 이동하고 페이지별 의미 assertion 추가.
- 설명만 있던 전체 지우기·승인 상세·이전 질문·답변 검토·복구 동작:
  제안 키와 문맥 행동 메뉴, 전체 답변 검토 장면을 추가. 기존 키 충돌과 미구현 상태는 README에 명시.
- 변경 provenance 누락/혼동: Proposed·Recorded·Reported·Current를 별도 장면으로 구별하고,
  Recorded에 완료 편집의 before/written 비교 근거를 표시.
- process 승인에 source 파일 편집이 섞이던 문제: 실행 요청의 효과와 쓰기 범위를 일치시키고 편집 요청을 분리.
- 정리 실패·재시도·완료가 한 화면에 섞이던 문제: 독립 상태와 행동으로 분리.
- 긴 명령·미완료 Markdown·slash palette·좁은 표 witness 부족:
  실제 긴 argv, 부분 응답, 검색 결과, 폭별 동일 값의 표/목록 예시를 추가.
- 빈 검색 결과와 빈 저장소의 문구 혼동, 검색 개수와 항목 불일치, 갤러리 fragment/history 불일치 수정.

## 실행한 검증

| 검사 | 결과와 의미 |
|---|---|
| `python …/tui-ansi/generate.py --check` | PASS. 결정적 생성 결과/해시/SGR-only/ANSI→TXT 동등성, 고정 행·열, critical state/action의 페이지별 유지 |
| libc `wcwidth`, `C.UTF-8` 독립 계산 | TXT **9,278개 행**, 열 수 불일치 0. generator와 다른 폭 계산 사용 |
| 64개 상위 + 10개 하위 ID 대조 | 누락/미등록 ID 0. **목차 연결 검사**이며 전 상태의 시각·행동 검증이 아님 |
| 소스/링크 검사 | P/C 24개 source 파일 존재, 최종 로컬 문서 경로 존재 확인 |
| Node `--check` | 갤러리 JavaScript 구문 PASS |
| Node DOM stub smoke | **522개 variant page**의 장면·폭·색·이전/다음·download 경로 PASS. 브라우저 layout 실행은 아님 |
| 갤러리 팔레트 대비 계산 | 지정 dark/light 배경의 12개 foreground 역할 최소 **6.06:1**. 임의 터미널 배경 대비 보장은 아님 |
| `python3 tools/context.py check` | PASS. Markdown route/local link 검사이며 의미 완전성 검사는 아님 |
| `git diff --check` | 기존 tracked diff 검사 PASS. ANSI/TXT의 오른쪽 셀 padding 공백은 고정 geometry를 위한 의도된 데이터 |

최종 manifest SHA-256:
`506c5dae043e062e3470c8a7585611c40e9126c63fe71408a9739272686e780c`

generator의 `--check`는 재현 가능한 검사다. 별도 libc/DOM stub/대비 계산은 이번 검수 중 실행한
보조 검사이며 실제 Yo 동작 테스트를 흉내 내는 production 테스트를 추가한 것은 아니다.
fixture 내용은 모두 예시이며 테스트 성공 숫자/사용량/시간도 실제 작업 실행 결과가 아니다.

## 남은 실제 제품 검증

- 브라우저 screenshot/레이아웃을 실행하여 검증하지 않았다. 터미널 폰트와 브라우저 폰트 차이가 있다.
- PTY 입력·실제 resize·focus/frame 수락·clipboard·외부 편집/suspend·SSH/tmux·screenreader를 실행하지 않았다.
- upstream 테스트나 모델/도구 실행을 수행하지 않았다. pin 비교이며 최신 릴리즈 조사 완료를 뜻하지 않는다.
- 58번 cursor/selection은 표기 예시이고 Unicode/IME/emoji 편집기의 실행 증거가 아니다.
- 정지 화면은 모든 상태 조합·전이·큰 출력의 성능·승인 병렬 실행을 증명하지 않는다.

이후 구현은 [요소별 동적 검증 목차](coverage.md)를 따라 실제 owner/accepted contract/테스트를 연결해야 한다.
이번 변경은 연구 자료와 연결 문서만 추가하며 product TUI 동작과 authority를 변경하지 않는다.
