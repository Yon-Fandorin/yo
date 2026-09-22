# 터미널 환경 매트릭스

터미널 동작의 기준은 실제 PTY 출력이다. HTML fixture는 진단과 parity
review를 돕지만 이 검사를 대신하지 않는다.

## 일반 test가 검사하는 범위

Linux에서 ignored가 아닌 `yo-cli` test는 실제 PTY를 만들고 Inline 종료,
Fullscreen 종료, signal에 의한 복원, 두 mode의 두 번 연속
`Ctrl+Z`/`SIGCONT` 세대를 실행한다. tmux, `sshd`, 설치된 Codex는
필요하지 않다.

```bash
cargo test -p yo-cli pty_tests::
```

process coordinator test는 handler 설치, rollback, shutdown compensation,
thread ownership, 격리된 subprocess signal 동작을 별도로 실행한다.

```bash
cargo test -p yo-cli execution::process::termination::tests
```

이 호스트 통합 검사는 일반 package test에 포함된다. 통과했다고 해서 tmux나
SSH 동작까지 실행되었다는 뜻은 아니다.

## 저장된 Mac의 큰 본문 페이지 검증

2026-09-13에 clean 후보 `af16336de8836475af39022d08bf612d13955110`이
저장된 Apple Silicon Mac의 `yo-tui` 검증 profile을 통과했다. 정확한 Git
번들을 확인하고 pinned Rust toolchain으로 임시 checkout에서 빌드했다.
Frame 테스트 6개, 크기 0 재진입 테스트 2개, all-target TUI 테스트 921개와
통합 테스트 4개, all-target Clippy `-D warnings`가 통과했다. 개별 7만 행
본문, 큰 표 값과 펼친 diff, 좁은 폭의 탐색, 완전한 페이지별 Inline 게시의
실패 복구를 포함한다. 이 자동 Mac 검사는 실제 IME나 붙여넣기 키 검증을
반복한 것은 아니다. 설치본과 정상 인증정보는 유지했고 소유한 checkout,
번들과 runner는 제거했다.

## OpenRouter 이미지 사용 흐름

[명시적 무료 이미지 정의](../workflows/provider-catalogs/openrouter.md#명시적-무료-이미지-연결)와
격리된 설정·인증·Session state를 사용한다. Private socket Ctrl+V, 첨부 미리보기,
제출·스트리밍 완료, 연결된 로컬 도구 결과, idle `/compact`, 정상 종료와 새
`--continue` 응답을 확인한다. 모든 요청의 PNG occurrence, NVIDIA 전용 경로,
가격 상한 0과 fallback 비활성화를 확인한다. 단위 test와 로컬 fixture는 전송·복원
검증이며 실제 Provider 실행은 실제 서비스 검사가 필요하다.

2026-09-13 실제 Linux PTY의 Fullscreen TUI로
`nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free`에서 다음을 확인했다.
표에는 아래에 설명한 최종 제한 요약 충실도 검증 결과를 반영했다.

| 검사 | 결과 |
|---|---|
| 정의 가져오기·private socket의 합성 64 × 32 PNG | 통과 |
| 왼쪽 빨강·오른쪽 파랑 설명·스트리밍 Turn 완료 | 통과 |
| 실제 `read_file` 1회·독립 확인한 marker와 연결된 결과 | 통과 |
| 두 번째 Turn·이미지 인식 idle summary checkpoint | 통과 |
| 요약 후 새 프로세스 재개·이미지 색상 유지 | 통과 |
| 실제 요약 뒤 최초 marker 정확히 반환 | 최종 제한 실행에서 통과 |
| 최종 정상 TUI 종료·termios 복구 | 통과 |

이 이미지 profile의 함수 도구 선택은 명시적 `tool_choice: auto` 없이 자동
기본값을 사용한다. 의미적 종료 뒤 동일한 response id, index 0, 반복된 종료
이유, 비어 있는 닫힌 role/content delta인 최종 accounting choice만 받는다.
usage를 한 번 기록하고 출력 중복이나 이전 종료 상태 변경은 허용하지 않는다.
`[DONE]`, 합계 검증, 한도와 중복·후행 데이터 거절은 유지한다.
Connector test 34개와 managed backend test 84개가 통과했다.

이전 진단에서 marker 누락, 중단된 응답의 최종 usage 미확인, HTTP 200 안의
SSE 오류 502를 관찰했다. 오류만으로 replay 직렬화 결함을 확정하지 않는다.
아래 최종 실행은 해당 제한 시나리오의 요약 충실도를 검증한다. CLI는 같은 실행
identity의 checkpoint-only 후보를 읽기 전용 복구로 검증한 뒤 선택하며, 미지원·
unavailable·다른 identity는 승격하지 않는다. 오프라인 정상·embedded 502
실제 PTY 대조군은 외부 추론 없이 정확한 checkpoint·최종 Journal 일치와 termios
복구를 확인한다. HTTP status나 `[DONE]`만으로 성공을 판정하지 않는다.

설정된 Mac은 profile compile·import, tmux 첨부와 SSH PTY lifecycle 검사를
통과했다. 물리 키보드·IME·실제 Command-V는
[직접 입력 검증](#현재-mac-직접-입력-검증)에서 통과했다. Mac 검사는 터미널
동작 검증이며 위 실제 Provider 검사는 Linux에서 수행했다.

### 이미지 요약 충실도 검증 완료

사용자가 요청한 새 검증은 깨끗한 candidate `0197d6f4`에서 허용된 최대 4회
요청을 모두 완료했다. 같은 NVIDIA 단일·가격 상한 0 이미지 경로를 유지했다.
일반 바이너리 SHA256은
`22cb8cb9587a5fd1eebbb31e67372f8d3dc3cb73324c5b4afdbb95daa7e898a0`다.
요청 전에 현재 공식 모델·endpoint 목록에서 이미지 입력, 함수 도구와
prompt/completion 가격 0인 유일한 NVIDIA 경로를 확인했다. Chat Connector
test 34개와 managed backend test 85개가 통과했다. 같은 바이너리의 오프라인
정상·embedded SSE 502 대조군도 외부 추론 없이 통과했다.

| 요청 | 입력 토큰 | 출력 토큰 | 전체 토큰 | 보고 비용 |
|---|---:|---:|---:|---:|
| 이미지 입력·초기 식별자 | 1278 | 143 | 1421 | 0 |
| 두 번째 Turn | 1337 | 24 | 1361 | 0 |
| 이미지가 포함된 idle 요약 | 621 | 1344 | 1965 | 0 |
| 새 프로세스 재개 | 1277 | 240 | 1517 | 0 |
| 합계 | 4513 | 1751 | 6264 | 0 |

모든 stream에서 semantic `stop`, 필수 `[DONE]`과 최종 usage를 확인했고
도구 호출은 없었다. 최초 요청과 요약 요청에는 canonical PNG가 각각 포함됐다.
식별자 `YO_FIDELITY_LITERAL_6D82A31F`가 요약 입력·출력, 저장된 checkpoint,
재개 입력과 최종 완료 Journal message에 정확히 있었다. Checkpoint의 portable
body는 요약 응답 전체와 bytes가 일치했고 재개 요청에도 그 요약 전체가 있었다.
최종 Journal bytes는 최종 응답과 일치했으며 답변에는 식별자와 이미지 색상명
빨강·파랑이 유지됐다. 서로 다른 Turn 3개와 checkpoint 1개가 완료됐고
두 TUI 프로세스 모두 status 0으로 종료하며 termios를 복구했다.

이 제한 시나리오로 미완료였던 실제 이미지 요약 충실도 검증을 닫는다. 위의
이전 marker 손실·502 관찰을 없애거나 모든 요약의 충실도와 이후 무료 경로의
가용성을 보장하는 결과는 아니다. 자동 재시도·fallback·경로 상한 변경·추가
추론 요청은 없었다. 중계·합성 파일·격리된 인증·Session state·TLS key를 제거했다.

## QwenCloud 재구독과 이미지 기능

2026-09-13 candidate `9eb0bbf9`에서 기존 인증된 QwenCloud 계정을 새로 조회해
재구독된 Standard Token Plan을 확인했다. 이 조회는 일반 계정 용량 cache를
갱신했으며 추론 요청은 아니었다. QwenCloud provider test 12개와 Responses
connector test 57개가 통과했다.

직접 Responses API 요청 1회는 설정된 `qwen3.8-max`와 정확한 국제 Token Plan
endpoint를 사용했으며 redirect·재시도·종량제 fallback은 없었다. 합성 64 × 32
PNG의 왼쪽 빨강·오른쪽 파랑에 대해 HTTP 200, status `completed`인
`response.completed`, 정확한 색상 답변을 확인했다. Response
`resp_ebd49743-e297-4633-ad04-7b4f0154f692`의 보고량은 입력 148·출력 56,
합계 204 token이었다. 사용자가 승인한 구독을 사용했으며 비용 0이나 일반 API
무료 쿼터 사용을 주장하지 않는다.

이는 Provider API의 이미지 기능만 통과한 결과다. Yo의 일반 API 이미지 사용 흐름은
아래 별도 검증을 참고한다.
일반 API 무료 쿼터는
[Token Plan 쿼터와 별개다](https://www.alibabacloud.com/help/en/model-studio/new-free-quota).
결과 기록 후 임시 기능 probe는 제거했다.

## QwenCloud general image journey

[명시적 일반 API 이미지 연결](../workflows/provider-catalogs/qwencloud.md#explicit-general-api-image-connection)을
사용해 stock Fullscreen TUI의 실제 Linux PTY에서 private socket Ctrl+V·미리보기·
제출·function tool 실행·idle `/compact`·정상 종료·새 `--continue`를 검증한다.
현재 승인된 정확한 Flash envelope를 사용하고 plan 계정이나 다른 model로 대체하지 않는다.

2026-09-13 `qwencloud:general:qwen3.8-flash`와 국제 Chat endpoint에서 최대 5회
요청을 모두 완료했다. 인증된 읽기 전용 quota 확인에서 `Free quota only`는 실행 전후
켜져 있었고 무료 잔여량은 988,173에서 982,591토큰으로 감소했다. 이 차이는 완료된
usage 합계와 정확히 같다. Provider 설정 변경·redirect·재시도·fallback은 없었다.
응답에 비용 필드는 없어 보고 비용을 0이라고 주장하지 않는다.

| 요청 | 입력 토큰 | 출력 토큰 | 전체 토큰 |
|---|---:|---:|---:|
| PNG·실제 파일 tool 호출 | 1024 | 48 | 1072 |
| 연결된 tool 결과·첫 답변 | 1139 | 22 | 1161 |
| 두 번째 Turn | 1207 | 22 | 1229 |
| 이미지가 포함된 idle 요약 | 589 | 265 | 854 |
| 새 프로세스 재개 | 1245 | 21 | 1266 |
| 합계 | 5204 | 378 | 5582 |

합성 64 × 32 PNG의 왼쪽 빨강·오른쪽 파랑을 독립 확인했다. 실제 `read_files`의
call id와 완료 결과를 연결하고 workspace의 정확한 marker 내용도 확인했다.
`YO_QWEN_IMAGE_LITERAL_81C64E2A`와 두 색상이 요약 source·응답·저장 checkpoint·
재개 입력·최종 봉인된 Journal까지 유지됐다. Checkpoint 본문은 전체 요약 bytes와
같았고 재개 입력은 그 요약을 그대로 포함했으며 최종 Journal도 최종 응답 bytes와 같았다.
앞선 요청 4개에는 같은 canonical PNG가 각각 1회 있었고 재개 요청에는 없었다.

요청 5개 모두 두 thinking 옵션이 정확한 boolean `false`였다. 활성 tools는 명시적
`tool_choice: auto`, 요약은 tools와 tool_choice 생략을 확인했다. 양수 output cap은
매번 131072였다. 모든 stream은 index 0, semantic finish, 최종 usage, 필수 `[DONE]`을
완료했다. 서로 다른 Turn 3개·checkpoint 1개가 완료됐고 두 프로세스는 exit 0,
Fullscreen 해제와 정확한 termios 복구를 확인했다. Checkpoint 전후 네 accounting
필드를 보존했고 압축 후 이미지가 없어도 Qwen advisory policy와 reserve 0을 유지했다.

같은 바이너리의 오프라인 정상·HTTP-200 embedded-502 대조군은 외부 추론 0회로
통과했다. 실패 Turn을 정상 응답으로 집계하지 않았다. Chat Connector 39개,
managed backend 85개, core model service 176개와 전체 workspace·Clippy가 통과했다.
바이너리 SHA256은
`ef9462786b6fb7dfccc40ad789bfa1a09440d3a880dcf54ffd60602f3ff4db46`다.
실제 key는 일반 Yo 인증 저장소에서 메모리로만 읽었고 격리된 Yo는 합성 로컬 key를
사용했다. 정상 일반 API key와 별도 Token Plan 계정은 보존했다. 임시 설정·Session·
socket·합성 파일·중계·TLS key는 결과 승격 뒤 제거했다. 이 검증은 해당 제한
시나리오의 근거이며 모든 향후 요약이나 무료 쿼터 가용성을 보장하지 않는다.

Native Codex 0.154.0이 구현 커밋 `d0a627cc`를 `gpt-5.6-sol`, effort `high`로
검토하고 Session `01a09ac4-b2b8-7e20-b2a5-c201131c69b4`에서 `CLEAR`를 반환했다.
첫 호출은 응답 본문 decoding 중 실패해 verdict가 없었다. 사용자는 같은 고정
입력 285,804 bytes(SHA256
`e366488e9fd3258b262f97ff86a830e566e0779e0ea9ab9fae9b5f53c4a2fcc5`)을 새 Session으로
추가 1회 전송하도록 승인했다. 전체 native 호출은 2회였고 reviewer tool 호출·
자동 재시도·steer·fallback은 없었다. 완료된 검토는 입력 70,502·출력 2,077 token을
보고했으며 호스트 비용은 보고하지 않았다. Finding-resolution round는 없었다.

## QwenCloud 관리형 비밀 입력

2026-09-22 수정되지 않은 후보 `2eace350`과 Linux 원본 바이너리 SHA-256
`fec70a33dcdb089c63e371ba6478665a606f7afb6dda08645326f78bdf4b63a4`에서
기존 `qwencloud:general:qwen3.8-flash` Chat binding으로 격리된 tmux TUI의
`request_secret_input` Turn 1개를 완료했다. 추론 전 읽기 전용 계정 조회에서 정확한
Flash 무료 쿼터가 유효하고 `Free quota only`가 켜져 있음을 확인했다. 일반 Yo 인증
정보는 원래 저장소에서 읽었고, 임시 Session 상태와 전용 tmux 터미널만 생성했다.

초기 프롬프트는 `Please ask me for a pretend word using
request_secret_input. Use title Test word, question Enter any made-up word,
and purpose Check the input UI. After the result, reply
YO_MANAGED_SECRET_OK.`였다. 모델은 비밀 입력을 정확히 한 번 요청했다. Yo는 숨김 입력
전에 선택된 Provider와 Model을 고지했다. 합성 값을 한 번 입력한 후 보호된 후속 응답은
그 값을 되풀이하지 않고 정확히 `YO_MANAGED_SECRET_OK`를 반환했다. Request audit에는
수락된 모델 요청 2개가 기록됐다. 완료된 사용량 영수증은 각각 입력 1,231/출력 57 및
입력 163/출력 5 token으로 총 1,456 token이었다. 원본 실행 전후 읽기 전용 무료
쿼터 조회에서 잔여량은 967,478 -> 966,022 token으로 정확히 같은 양만큼 감소했고
`Free quota only`는 계속 켜져 있었다. Journal에는 값 없는 제출 영수증 한 개와 완료된
Turn이 남았으며, 합성 값은 Journal·캡처된 TUI·stderr에 없었다. 새 프로세스의
`--resume`은 이 Session이 보호 입력에서 끝났다는 이유로 추론 전에 거부됐다. TUI와
전용 tmux 서버는 종료됐고 일반 인증 정보와 설정은 변경되지 않았다. 이 검증은
tmux로 키 입력 이벤트를 주입한 Linux 검사이며 Mac 실제 키보드 검사는 아니다.

앞선 강한 표현의 테스트 프롬프트는 제출 뒤 Qwen의 `data_inspection_failed` 코드와
HTTP 400을 받았다. 비밀값을 가려 확인한 Yo 요청에는 서로 일치하는 assistant
도구 호출과 결과가 한 쌍 있었고, `tools`·`tool_choice`는 빠져 있었으며 승인된
binding 옵션을 사용했다. 직접 비교에서 결과를 중립 문자열로 바꿔도 같은 요청은
거부됐지만, 최초 사용자 문구만 중립적으로 바꾸면 수락됐다. 따라서 해당 테스트
내용에 대한 제공자 입력 검사 거부였으며 Yo의 호출 연결 오류는 아니었다. 다른
비밀값에 대한 제공자 판단까지 예측하지는 않는다. 성공을 위해 제공자 설정·대체
경로·재시도·redirect·Yo 계약을 변경하지 않았다. 무료 쿼터 잔여량이나 사용량
영수증만으로 비용 0을 주장하지 않는다.

## 위임형 비밀 입력 검증 범위

2026-09-22 현재 무료 모델을 사용한 Mac 실서비스 Turn에서 위임형
`UserInputRequest`의 `isSecret: true`는 입증되지 않았다. Yo의 Codex 어댑터는
해당 wire 형태를 결정적 테스트로 검증하지만, 설치된 Codex 0.155.1의 모델용
`request_user_input` 도구에는 `isSecret`이 없고 비어 있지 않은 선택지가
필요하다. 비동기 질문 도구는 이 요청 대신 일반 메시지를 만든다. 설치된 Grok
1.0.40의 사용자 질문 경로에는 지원되는 비밀 표시가 없으며, 기존 실서비스 검증에서 확인한
인증된 Grok 모델 경로는 구독 용량을 사용했다. 따라서 두 호스트의 기본 질문
경로만으로는 이 정확한 무료 모델 실서비스 여정을 시작할 수 없다. 위의 관리형
QwenCloud 결과를 위임형 호스트 검증으로 간주하지 않는다.

Codex의 선택형 진단 도구 `yo_secret_entry_probe`는 `YO_CODEX_SECRET_ENTRY_PROBE=1`이고
app-server 버전이 정확히 0.155.1일 때 새 일반 Session 생성 시 등록된다. 실험적
`dynamicTools`를 사용해 Yo가 작성한 고정 질문으로 기존 숨김 입력에서 **모의값만**
받는다. 도구는 모델이 작성한 질문 인자를 받지 않으며 시작된 호출 하나당 입력창은
한 번만 열 수 있다. Yo는 입력값을
폐기하고 고정된 공개 완료 상태만 반환하므로 모델은 입력값을 사용할 수 없다.
로컬 app-server 0.155.1은 모델 Turn 없이 빈 인자 동적 도구 등록을 수락했고,
initialize 응답의 실제 user-agent 형식은 `yo/0.155.1`이었다.
결정적 어댑터 테스트는 합성 `item/tool/call`로 숨김 질문을 열고 모의값이 Codex
응답에 없음을 확인했다. 등록만으로는 질문이 열리지 않는다. 실제 모델 Turn이
모델에 보이는 이 도구를 선택하거나 신뢰할 수 있는 모의 호스트가 일치하는 호출을
보내야 한다. 일반 작업에서는 선택 설정을 끄고, 설정이 켜진 동안에는 해당
Session의 모델 Turn에 도구가 제공됨을 유의한다. 이는 프로토콜·어댑터 검사이며
Mac 실서비스 Turn이나 실제 자격증명 사용 도구의 증거는 아니다. Codex는 해당
Thread를 재개할 때 도구 정의를 보존할 수 있으며, Yo는 선택 설정이 켜져 있고
wire 버전이 여전히 정확히 0.155.1일 때만 호출을 처리한다. 실제 모델 요청 전에는 무료
사용 자격을 확인해야 한다.

Grok 어댑터에도 별도의 선택형 진단이 있다. 일반 Session에서
`YO_GROK_SECRET_ENTRY_PROBE=1`을 사용한다. Grok ACP가 HTTP MCP를 지원한다고
선언해야 하며, Yo는 `session/new`와 `session/load`에 루프백 전용의 임의 주소
MCP 서버를 연결한다. 이 서버는 인자 없는 `yo_secret_entry_probe` 도구를
제공한다. 호출하면 Yo가 작성한 고정 숨김 질문에서 **모의값만** 받고, 입력값을
폐기한 뒤 고정 상태만 반환한다. 비어 있지 않은 인자가 있는 호출, 활성 Turn 밖의 호출,
동시 호출, 읽기 전용 검토에서는 입력창을 열지 않는다. Grok의 일반
`_x.ai/ask_user_question` 경로에 비밀 표시를 추가하거나 실제 자격증명을
전달하는 기능은 아니다. 일반 작업에서는 선택 설정을 끈다. 로컬 어댑터
테스트는 MCP에서 숨김 입력으로 이어지는 경로와 MCP·ACP 응답에 모의값이
없는 것을 확인한다. 모델이 실제로 도구를 선택하는 검사는 별도다.

2026-09-22 저장된 Apple Silicon Mac에서 고정 호스트 키와 읽기 전용 도구
사전 점검이 통과했다. 설치된 버전은 동일한 Codex 0.155.1과 Grok 1.0.40이었다.
모델 프롬프트 없이 Grok 1.0.40이 HTTP MCP를 지원한다고 선언하고 ACP
`session/new`에서 임시 루프백 테스트 서버에 접속해 `server/discover`,
`initialize`, `tools/list`를 수행하는 것도 확인했다. 이는 진단 도구의 전송
경로 증거이며 모델이 비밀 질문을 호출했다는 뜻은 아니다.
`grok models`에는 `grok-4.7`, `grok-4.7-build-fast`, 기본값인 `grok-4.6`,
`grok-4.5`가 있었지만, 이 목록만으로 무료 사용 자격을 입증할 수 없다. Mac에는
이 사전 점검에서 모델 프롬프트·비밀값을 보내지 않았다. 호스트에 접속할 수 있게
된 것만으로 위임형 모델 호출 결과가 바뀌지는 않았으며, 상태는 미검증이다.

이후 검토된 후보 `e7811016`은 Mac 임시 체크아웃의 Grok 검사에서 통과했다.
어댑터 테스트 92개가 통과했고(설치 호스트 전용 3개는 평소 제외), 패키지 Clippy와
Yo CLI 검사도 통과했다. 별도 실행한 선택형 설치 호스트 테스트에서는 Grok
1.0.40 인증 후 Yo의 로컬 HTTP MCP 서버를 붙인 ACP Session을 만들고 모델
Turn 없이 종료했다. 동일한 설치 호스트 테스트는 로컬 Linux에서도 통과했다.
검증 실행기는 Mac의 임시 체크아웃과 번들을 제거했고 재사용하는 객체 캐시만
남겼다. 진단 도구에 실제 자격증명을 입력하거나 Grok에 반환하지 않았으며,
호스트 인증에는 기존 캐시된 로그인을 사용했다. 설치된 모델 목록으로
무료 Build 사용 자격을 확인할 수 없고 실서비스 검증은 무료 사용으로 제한되어
있으므로, Grok 모델이 진단 도구를 실제로 선택하는 단계는 아직 검증하지 않았다.

## QwenCloud 무료 텍스트 요약과 재개

2026-09-13 인증된 계정의 쿼터를 읽기 전용으로 조회해 `qwen3.8-max-0902`를
선택했다. 해당 쿼터는 유효하고 검증 시작 전 1,000,000 token이 남아 있었으며
[`Free quota only`](https://docs.qwencloud.com/resources/free-quota)가 켜져 있었다.
별도 일반 API key와 정확한 국제 Responses endpoint를 사용했고 설정·workspace·
Session을 임시로 격리했다. Proxy가 권한을 제한한 임시 파일에서 실제 key를 읽었고
Yo에는 합성 로컬 인증 정보만 전달해 Yo 설정에 실제 key를 넣지 않았다.

첫 bounded run은 `29fc2b9c`에서 최대 4회 중 3회 요청 후 중단됐다. 일반 응답
2회는 완료됐지만 요약 처리에서 추론 output slot 0 뒤의 답변 slot 1을 거부했다.
checkpoint는 저장되지 않았으며 중단된 요약의 완료 여부와 usage는 알 수 없다.
수정 candidate `8afc960f`는 자동·idle 압축 모두 유일한 요약 메시지를 output slot과
item ID로 함께 식별한다. 다른 메시지·추가 content part·완료 identity 불일치·중복
완료·완료 뒤 텍스트는 계속 거부한다.

수정본 검증은 초기 답변·두 번째 Turn·tools-disabled 요약·새 프로세스의
`--continue` 답변을 포함한 요청 4회가 모두 통과했다. 모든 응답은 HTTP 200,
완료 terminal과 최종 usage를 보고했다. 정확한 초기 reference와
`LEFT=red RIGHT=blue` 사실이 요약 source·요약·저장된 checkpoint·재개 요청·최종
답변까지 보존됐다. checkpoint 본문은 요약 bytes와 같았고 최종 Journal의 봉인된
답변도 response bytes와 같았다. 요약
`resp_8473fe12-7673-9f6d-84fe-b37a6a479f79`와 재개 답변
`resp_fb9e33e9-8724-9496-883c-4b889bfb9c4d`는 각각 합계 7,954·1,510 token을
보고했으며 완료 응답 4회의 합계는 23,080 token이었다. 중단된 첫 run과 수정본
run의 실제 요청은 총 7회였고 redirect·자동 재시도·다른 model·구독 fallback은 없었다.

Managed backend test 85개가 모두 통과했다. 자동·idle 압축과 disk resume에서
답변 slot 0·1을 검사하고 두 경로 모두 잘못된 event 7종을 거부했다. 실제 PTY의
로컬 성공 및 HTTP-200 실패 대조 검사는 추론 요청 없이 통과했다. Native Codex
0.154.0은 `gpt-5.6-sol`, effort `high`, Session
`01a09945-2a3f-76c0-bf0e-2e86bfc24530`에서 정확한 구현 candidate를 독립 검토해
지적 없이 승인했다. 검토 1회·reviewer tool 호출 0회·finding-resolution round
0회였다. 빌드·formatting·workspace Clippy도 통과했다. 검증한 바이너리의 SHA-256은
`78fa40e59ac859de2463b0474450cb35c8a5336dfd9bffccde66150ad2fa7c22`였다.

실제 TUI 종료 2회는 exit 0이며 터미널 설정을 복원했다. Proxy·이번 probe 상태·
임시 key·검토 패킷은 제거했고 실제 검사 중 일반 Yo/Codex 설정·인증 파일은
바뀌지 않았다. 공통 텍스트 요약·checkpoint·replay 경로는 통과했으며 이미지 인식
요약 충실도는 이 페이지의 OpenRouter·QwenCloud 별도 검증을 참고한다.

## Managed 요청 실패 진단

Provider 계정 없이 요청 수락 전 실패 경로를 결정적인 fixture로 검사한다.

```bash
cargo build --locked -p yo-cli
python3 tools/validation/managed-start-failure.py target/debug/yo --mode inline
python3 tools/validation/managed-start-failure.py target/debug/yo --mode fullscreen
```

[Runner](https://github.com/Yon-Fandorin/yo/blob/develop/tools/validation/managed-start-failure.py)는
가짜 키와 loopback 전용 endpoint를 사용하는 text-only Chat Completions binding을
가져온다. 로컬 socket은 HTTP 이전의 TLS 연결을 거절하고, 트래픽을 전달하거나
인증서 신뢰 설정을 바꾸지 않는다. 각 mode에서 한 번 제출한 뒤 종료 코드 1,
typed `Transport` stderr, 저장된 `last_failure.kind: transport`, 수락된 요청 0건,
완료된 Turn 0건, PTY mode 복원을 검사한다. Fullscreen에서는 alternate-screen
진입·종료 쌍도 검사한다. Parent가 복원된 mode를 수집할 때까지 shell이 controlling
terminal의 session을 유지하며, private pipe로 수집 완료를 알린다. Darwin에서는
canonical input 복귀 시 kernel이 설정하는 pending-input 상태 비트 `PENDIN`만
비교에서 제외한다. 이는 [Apple의 tty 구현](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/tty.c)에서
확인할 수 있다. 나머지 termios 필드는 모두 비교한다. 격리된 설정·credential·host identity·Session을 지우기
전에 JSON 진단을 수집하며, 검사 실패 시에도 정리 결과를 보고한다. 일반 사용자
상태는 사용하지 않는다.

2026-09-12 Linux의 두 mode가 모두 통과했다. Loopback 연결 시도 1회, 종료 코드 1,
typed stderr와 저장된 failure, 수락된 요청·완료된 Turn 0건, terminal 복원을 확인했다.
의도적으로 import를 실패시킨 검사도 실패 진단을 반환하고 임시 상태를 제거했다.
상태 코드만 보존하는 HTTP 실패 분류를 포함해 공통 transport 검사 13개도 통과했다.

같은 날 후보 `0f86d85e`가 macOS 26.6.2 arm64에서 빌드됐고 Inline·Fullscreen 모두 통과했다.
각 mode에서 제출 1회, loopback 연결 시도 1회, Yo 종료 코드 1, typed `Transport`
stderr, 저장된 `transport` failure를 수집했다. HTTP 요청·수락된 요청·완료된 Turn은
모두 0건이었다. 두 mode 모두 kernel의 `PENDIN` 상태 비트를 제외한 모든 PTY 설정을
복원했다. Fullscreen은 alternate-screen 진입·종료 쌍을 정확히 한 번 출력했고
Inline은 출력하지 않았다. 각 격리된 상태 root와 임시 checkout을 제거했으며 일반
설정·credential hash는 바뀌지 않았다. 이로써 공통 요청 수락 전 실패 진단의 Mac
검증은 완료됐다. 이 fixture는 OpenRouter inference 요청을 보내지 않았다. 이전
실제 서비스 종료의 구체적인 원인은 미검증이며, 위의 별도 이미지 실제 서비스
여정은 제한된 시나리오에서 통과했다.

`BackendRequestAccepted`는 connector 시작이 성공한 요청 수다.
[Transport worker](https://github.com/Yon-Fandorin/yo/blob/develop/crates/connectors/transport/src/worker.rs)는
HTTP 전송 뒤 상태 코드나 transport 오류를 반환하여 이 기록이 저장되기 전에
실패할 수 있다. CLI는 오류를 전달하고 terminal을 복원한 뒤 종료한다. 제출,
연결·HTTP 시도, 수락, 완료된 Turn 수를 구분한다. 실제 서비스 harness를 정리하기
전에 child 종료 코드, secret-safe stderr, binding의 typed failure를 수집하고,
관찰이 끝날 때까지 tmux pane을 유지한다. 수락된 기록이 0건이라는 사실만으로
HTTP 시도도 0회였다고 판단할 수 없다.

이 fixture는 공통 실패·진단 경로를 검증하며 OpenRouter image binding이나 Provider의
HTTP 거절을 검증하지 않는다. 이전 Mac 실제 서비스 실행의 구체적인 원인은 삭제된
capture에서 복구할 수 없으므로 미검증으로 남는다.

## 설치된 Codex 검사

model Turn 없이 stdio initialize와 shutdown 경계를 검사한다.

```bash
cargo test -p yo-core local_codex_initializes_and_shuts_down \
  -- --ignored --nocapture --test-threads=1
```

버려도 되는 workspace에서 인증된 model Turn 하나, tool 실행, 파일 변경,
semantic event, 명시적 cleanup을 검사한다.

```bash
cargo test -p yo-core local_codex_completes_a_real_file_change \
  -- --ignored --nocapture --test-threads=1
```

두 번째 검사는 외부 model 연산을 수행한다. Turn 대기는 최대 180초이며
전체 실행 시간에는 Codex 시작과 종료도 포함된다. 호환되는 Codex 인증과
쓰기 가능한 Codex 상태가 있는 환경에서만 실행한다.

위임 Codex image 경계는 adapter와 core의 offline test로 검증한다. 정확한
`0.153.4`와 `0.154.0` wire 증거, 선택한 model의 정확한 `inputModalities`, start와 steer 양쪽의
순서가 있는 반복 immutable PNG projection, 보수적인 inherited-history resume/rebind
admission, 완전한 32 MiB outbound JSONL 경계를 검사한다. 경계 test는 정확히 32 MiB를
허용하고 첫 초과 byte를 peer에 보내기 전에 거절한다. server response도 같은 경계를
사용하며 overflow는 protocol failure로 분류한다. 이 검사는 fake JSONL peer를 사용하고
model service에 접속하지 않는다.

2026-09-10에는 `gpt-6-astra`를 사용한 격리된 공식 Codex 실행에서도 합성 model
요청 두 번으로 이미지 Turn과 새 프로세스 재개를 검증했다. 클립보드 fixture에 대해
정확한 `red,blue` 답변을 받았고, 재개 Turn도 정확한 회상 검사를 통과했다.
continuation anchor 두 개와 기존 journal prefix를 보존했다. 검사한 바이너리 SHA256은
`a20df491ff6445f521bb45d8f03f67f79fb7f72dbea355312ac36c9af394603d`다.
소유한 tmux Session을 종료하고 임시 인증을 제거했다. 이는 검사한 이미지 입력과
복구 경로를 입증하며 바깥 터미널의 이미지 픽셀이나 다른 model/version 조합을
검증하지 않는다. 승인된 요청 두 번의 예산은 모두 사용했다.

2026-09-11에는 외부 네트워크를 차단한 loopback 전용 namespace에서 설치된
Codex `0.154.0`의 이미지 wire 호환성을 검증했다. 설치된 `0.153.4`와 `0.154.0`
실행 파일로 생성한 schema bundle의 `TurnStartParams`, `TurnSteerParams`,
`ModelListResponse` 정의는 동일했다. 검토된 정확한 버전 목록은 두 버전을 포함하며,
인접 patch와 prerelease의 이미지 지원은 계속 Unknown으로 처리한다.

Native app-server는 1,080,993바이트 합성 PNG를 `turn/start`와 `turn/steer`에서
수용했고, 로컬 mock Responses 요청 두 번에서 이미지 바이트와 앞뒤 텍스트 순서를
보존했다. 별도의 실제 Rust Yo를 격리된 96×32 tmux에서 실행해 Ctrl+V로 이미지를
준비하고 Turn을 완료한 뒤, 종료하고 새 프로세스로 재개했다. Mock endpoint는
두 Turn 모두 Yo journal의 정확한 불변 PNG를 받았다(data URI 1,921,054바이트).
continuation anchor 두 개와 기존 journal prefix를 보존했다. 검사한 Yo 바이너리
SHA256은 `a4b4173d6ddba7ef9a092e960d229a93eaf028b315b43809749570d83a457aa5`다.

이 검사는 설치된 host와 합성 로컬 model endpoint를 사용했으며, 자격증명이나 외부
model 요청을 사용하지 않았다. 전송과 continuation 호환성을 입증하지만 실제 서비스의
시각 인식을 검증한 것은 아니다. Adapter suite는 선택 model의 modality admission과,
이미지가 남아 있는 대화의 대상이 text-only이면 native resume 전에 거부하는 동작도
검사한다. 기존 메시지 크기 제한과 불변 입력 검사는 계속 적용한다.

### 저장된 명령 규칙과 자동 승인

2026-09-11 Linux Rust yo를 격리된 110×44 tmux에서 실행해 공식 Codex `0.154.0`,
`gpt-6-astra`에 대한 합성 Turn 네 번을 검증했다. 바이너리 SHA256은
`f72d343be2d2aecef3f844033a6e84fa59aa3ffc902c680ced0a6b5bc6200d6a`다.
추론 전에 위임 Codex package 검사와 CLI build가 통과했다.

`approval_policy = "on-request"`, `approvals_reviewer = "user"`,
`sandbox_mode = "read-only"`에서 첫 Turn은 임시 스크립트 하나의 정확한 절대 경로에
대해 실제 TUI의 `Approve + save rule`을 선택했다. Native 규칙 파일에는 해당 allow
prefix만 저장됐고 스크립트는 marker를 정확히 한 번 추가했다. 승인 대기 패널을
40, 20, 110열로 변경하는 동안 실행이나 규칙 저장은 일어나지 않았다. 정상 종료 후
새 프로세스로 재개했을 때 같은 명령은 추가 승인 없이 marker를 한 번 더 추가했다.
기존 Yo journal의 물리적 prefix와 저장 규칙은 그대로 유지됐다.

다른 스크립트에는 새로운 승인 요청이 표시됐다. `Decline and stop`을 선택하자
출력 파일을 만들지 않고 저장 규칙을 유지하면서 Turn을 중단했다. 최초 드라이버는
이 취소된 suffix를 재개하려 했지만, 최신 durable anchor가 없어 현재 continuation
guard가 읽기 전용 기록을 열었다. 이 드라이버 순서 오류는 원래 기록에 보존했다.
완료한 세 시나리오는 재전송하지 않고 네 번째 시나리오는 새 Session에서 실행했다.

새 Session은 같은 read-only sandbox와 빈 규칙 디렉터리에서
`approvals_reviewer = "auto_review"`를 사용했다. 합성 append 한 번이 수동 승인 없이
완료됐다. 캡처한 `item/autoApprovalReview/completed`는 `decisionSource: "agent"`와
`approved`를 보고했고 명령 완료 전에 정확한 command item, thread, turn과 연결됐다.
파일에는 marker가 정확히 하나 있었고 규칙은 저장되지 않았다. 이는 호스트의
[자동 검토 경로](https://learn.chatgpt.com/docs/sandboxing/auto-review)를 검사한 것이며
승인 패널이 없다는 사실만으로 권한 부여를 추정하지 않는다.

오프라인 감사는 선택지 번호, 정확한 wire 결정, native 명령 결과와 Yo 요청·응답
신원을 대조했다. 선택지, 규칙 범위, cleanup, 요청 신원, 명령 상태, 검토 상태,
대상, 순서, 결정 출처, 검토 누락을 변조한 대조군 열 개를 모두 거절했다. 임시 인증,
native 상태와 규칙을 제거하고 작업 폴더를 비웠으며, 소유한 tmux 서버를 종료한 뒤
남은 소유 프로세스가 없음을 확인했다. `turn/start` 네 번은 내부 model이나 reviewer
요청 횟수를 뜻하지 않는다.

2026-09-13에 반복 가능한 Linux 명령
`python3 tools/validation/codex-policy-persistence.py /absolute/path/to/codex`으로
loopback 전용 사용자·네트워크 namespace에서 정확한 Codex `0.154.0`의 실제 정책
저장과 재시작 후 적용을 재확인했다. 호스트가 제공한 정확한 argv prefix의
`acceptWithExecpolicyAmendment`를 선택하자 native 규칙을 저장하고 소유한 marker에
한 번 추가했다. app-server 종료·새 프로세스 시작 후 `thread/resume`으로 같은 명령을
실행했을 때 재승인 없이 두 번째 marker를 추가했고 규칙 byte는 같았다. 다른 명령은
`cancel`을 제공했으며 Turn을 중단하고 marker와 새 규칙을 만들지 않았다. 로컬 합성
Responses 요청은 다섯 번이고 외부 모델 요청은 없다. `gpt-5.5`는 native fixture의 도구
구성을 선택할 뿐 OpenAI 모델 서비스를 호출하지 않았다. 정상 사용자 상태를 보존하고
소유한 임시 상태와 프로세스를 정리했다. 직접 native 검증으로 위 실제 Yo TUI 경로를
보완하며 영구 명령 거부 선택지를 추가하지 않는다.
실행기는 제공된 `cancel`, 실제 선택 기록과 `interrupted` 결과를 요구하고
fixture 요청이 정확히 다섯 번인지 검사한다.

별도 명령
`python3 tools/validation/codex-interview-resume.py /absolute/path/to/codex`은 같은
native 버전에서 비밀이 아닌 질문 두 개를 대기시킨 뒤 프로세스를 종료·재시작하고
디스크 thread를 복원했다. 예전 Turn은 `interrupted`였고 대기 중 질문 RPC는 재발행되지
않았다. 로컬 fixture 요청 한 번으로 외부 모델 요청 없이 확인하고 소유한 임시 상태를
정리했으며 정상 사용자 상태는 보존했다. 이 당시 검증으로 native 재개가 대기 중
질문을 재발행하지 않는다는 점을 확인했다. 현행 v1은 별도 로컬 초안을 저장하지만
종료된 provider 요청을 투명하게 이어 주지는 않는다.
실행기는 복원 응답 이후에도 최대 3초 안에서 1초 동안 이벤트가 없는 상태를
확인한다. 늦게 도착한 질문 RPC, 프로토콜 오류, reader EOF와 프로세스 종료는 검증 실패이며,
app-server가 살아 있고 이벤트가 없는 경우에만 통과한다.

### 자동 거절과 네트워크 승인 범위

2026-09-11에 같은 Yo 바이너리와 공식 Codex `0.154.0`을 격리된 120×48 tmux TUI에서
실행해 추가 Turn 열 번을 검증했다. 인증 정보 없는 로컬 Responses fixture가
결정적인 도구 호출과 reviewer 응답을 제공했다. 로컬 model/reviewer HTTP 요청은
20번이었고 실제 서비스 요청은 없었다. 이는 native 정책 실행, adapter 동작과
TUI 표시 결과를 검증하며 서비스 model의 위험 분류 정확도를 측정하지 않는다.

이 결정론적 fixture만으로 실제 서비스의 위험 분류를 증명하지 않는다. 아래 무료
Qwen 네이티브 host 검증이 정확한 해당 경로의 실제 분류 증거를 추가한다. 무료
OpenAI Codex 서비스 경로는 확인되지 않았다. 작업별 독립 코드 리뷰 모델 선택과는
별개다.

2026-09-13 별도 native Codex `0.154.0` app-server probe는 사용자가 승인한
재구독 QwenCloud Token Plan을 사용했다. Main agent의 도구 호출은 로컬 fixture이며
자동 검토는 명시적인 custom-provider catalog로 `qwen3.8-max`를 선택했다.
Offline 승인·거부 대조군은 정확한 검토·thread·Turn·명령과 독립 확인한 파일 결과가
일치했으며 실제 서비스 요청은 없었다.

실제 승인 사례는 검토 요청을 수정 없이 정확한 Qwen Responses endpoint로 1회
전달했다. HTTP 200을 관찰했지만 정상 분류 완료와 최종 usage는 확인하지 못했다.
두 번째 로컬 검토 요청은 실제 전달 없이 차단했다. Native host는 실패 시 실행을
차단했다. `decisionSource: "agent"`, `denied`, 위험도 high·승인 범위 unknown인
검토 뒤 같은 명령이 `declined`가 됐고 marker 파일은 생성되지 않았다. 이는 실패
처리 증거이며 Qwen 위험 분류 성공이 아니다. 실제 거부 사례나 다른 서비스로의
재시도·fallback 없이 실행을 중단했다. 임시 native state·로컬 더미 인증·script·
process를 제거했으며 일반 Codex 설정은 유지했다. 이 직접 native 검사는 새로운
Yo TUI 자동 검토 흐름을 검증하지 않는다.

#### 무료 Qwen 네이티브 자동승인 위험 분류

`06d3b4db`의 남은 작업 검증은 일반 API `qwen3.8-max-0902`의 유효한 무료 쿼터와
`Free quota only` 활성화를 확인했다. 공식 Codex `0.154.0`의 명시적인
custom-provider catalog와 reasoning effort `low`를 사용했다. Main agent는 로컬
합성 fixture였으며 수정하지 않은 guardian 요청만
`https://dashscope-intl.aliyuncs.com/compatible-mode/v1/responses`로 보냈다.
서비스 호환성 검증이며 독립 코드·문서 리뷰가 아니다. 독립 리뷰는 기존 인증된
Codex host를 계속 사용한다.

실제 허용 판단은 agent-sourced `approved`, low 위험과 unknown authorization을
반환했다. 같은 thread·Turn·target의 명령 완료보다 먼저 발생했고 명령 exit code는
0, 파일에는 정확히 한 번 추가한 marker만 있었다. 첫 observer가 SSE `data:` 뒤
공백을 가정해 완료·usage를 놓치고 이후 로컬 main-agent 응답도 중단했다. 해당
Turn은 승인된 명령 완료 뒤 로컬 harness 경계에서 실패했다. 네이티브 판단과 파일
결과는 실제 분류 증거지만 해당 요청의 최종 usage·전체 Turn accounting은 미확인이다.
허용 요청은 다시 보내지 않았다.

첫 거부 검증의 `resp_3abc1b14-3b0a-91d7-a682-37dae8b43400`은 판단 대신 읽기 확인
도구 응답을 정상 완료했으며 5,436토큰이었다. 1회 요청 제한으로 네이티브 후속 요청을
차단했으므로 그 결과의 실패 거부는 실제 분류에서 제외했다. 공백 없는 적법한
`data:` 필드 처리와 네이티브 읽기 확인 후속 경로를 오프라인으로 검증한 뒤,
별도로 고정한 수정 artifact의 거부 검증에 2회 요청을 허용했다. 첫 응답
`resp_43e22ea4-893c-94f4-bea2-2de42c8f74eb`은 읽기 확인을 완료하고 5,467토큰을
보고했다. 후속 `resp_859432c7-3ee1-98dd-929b-eed041447ae5`는 5,728토큰과 실제
`deny / high / unknown` 판단으로 완료됐다. Agent-sourced 네이티브 review
`ab30b414-c020-4229-a932-a8ce5fe84ad0`는 같은 thread·Turn·명령과 연결됐으며 명령의
`declined` 완료보다 먼저 발생했다. 해당 명령은 실행되지 않았고 Turn은 완료됐다.
금지된 upload는 가짜 credential 파일과 loopback에 고정한 예약된 invalid hostname을
사용했으며 실제 비밀정보나 외부 upload 대상은 없었다.

이 결과는 검사한 Qwen 경로의 실제 허용·거부 분류를 확인한다. 원래 2회 범위와
수정된 읽기 확인 2회 범위에서 총 4회 서비스 요청을 사용했다. 완료된 usage 3건의
알려진 합계는 16,631토큰이며 허용 요청의 usage는 미확인이다. 최종 검증의 두 번째
요청은 네이티브 읽기 도구의 후속 대화이지 HTTP·판단 오류 재시도가 아니다.
Redirect·HTTP 재시도·모델 변경·구독 fallback·수동 승인·저장 규칙은 없었다.
오프라인 허용·거부와 읽기 확인 대조군은 서비스 요청 없이 통과했다. 임시 native 상태,
로컬 가짜 인증, script·listener·process를 제거했다. 직접 네이티브 검증이며 새 Yo
TUI 흐름이나 다른 모델·Provider·행동의 위험 분류 정확도를 주장하지 않는다.

네트워크 검사는 native proxy를 활성화한 이름 있는 permission profile과 격리된
`[experimental_network]` requirements fixture를 사용했다. Mount namespace로 임시
requirements를 테스트 프로세스에만 제공했으며 실제 `/etc/codex`와 사용자의 Codex
설정은 변경하지 않았다. 대상은 수신 요청을 집계하는 loopback HTTP 서버였다.

- 제공된 Session 한정 승인을 선택하자 요청 하나가 도달했다. 실행 중인 같은
  Session의 다음 Turn은 추가 승인 없이 같은 대상에 도달했고, 새 프로세스와
  Session에서는 다시 승인을 요구했다. 취소했을 때는 대상에 도달하지 않았다.
- 제공된 영구 네트워크 허용을 선택하자 loopback 호스트와 HTTP protocol에 대한
  native `network_rule` 하나가 저장됐다. 새 프로세스와 Session에서도 추가 승인
  없이 적용됐다. 다른 호스트 이름에는 여전히 승인을 요구했고 취소하면 대상에
  도달하지 않았다.
- 명시적으로 준비한 네트워크 거부 규칙은 최초 실행과 새 프로세스 모두에서 승인
  요청이나 대상 도달 없이 접속을 차단했다. Yo는 정책이 도메인을 명시적으로
  거부한다는 native 실패 설명을 표시했다. 규칙은 변경되지 않았다.
- 주입한 reviewer 거부 응답은 native `denied` 검토와 `declined` 명령을 만들었다.
  Reviewer 응답을 멈추자 native의 90초 제한에 도달해 `timedOut`과 실패한 명령을
  만들었다. 둘 다 명령의 marker 파일을 만들거나 수동 승인을 요구하지 않았으며,
  Yo에 해당 거부 또는 시간 초과 설명과 함께 `Codex approval warning`을 표시했다.

Codex `0.154.0`은 metadata에 네트워크 amendment 두 가지를 제안했지만
`availableDecisions`에는 허용만 제공했다. 따라서 이 호스트 버전에서는 실제 TUI로
영구 거부를 저장하는 경로를 사용할 수 없다. 준비한 규칙 검사는 차단 적용을
입증하며 대화형 거부 저장 과정을 입증하지 않는다. Adapter 검사
`offered_approval_choices_preserve_exact_scopes_and_wire_payloads`는 호스트가 해당
선택지를 제공할 때 정확한 allow/deny 전송을 별도로 다룬다.

오프라인 감사는 열 번의 결과, 선택지 번호와 정확한 응답, 프로세스 경계, 저장
규칙, 대상 수신, 표시된 경고, 검토와 명령의 신원·순서를 확인했다. 증거를 변조한
대조군 여덟 개를 거절했다. 임시 native 상태, 테스트 규칙과 작업 폴더를 제거하고
소유한 프로세스와 tmux 서버를 종료했으며 loopback listener를 해제했다. Runtime
코드는 변경하지 않았으므로 앞선 package/build 검사 결과가 계속 유효하다.

## 설치된 Grok 검사

Session 생성이나 model 추론 요청 없이 ACP 초기화, cached-login 인증,
정상 종료를 검사한다.

```bash
cargo test --locked -p yo-backend-delegated-grok \
  runtime::tests::session::local_grok_authenticates_and_shuts_down_without_a_session \
  -- --ignored --exact
```

2026-09-10 Linux에서 Grok `1.0.25 (f7e67d6988e2)`로 이 검사가 통과했다.
독립 96×32 tmux 실행에서도 실제 Rust `yo --fullscreen --model host:grok`으로
빈 입력창까지 도달한 뒤 Ctrl+D로 정상 종료했다. 검사한 바이너리 SHA256은
`42f7c955d966d56825213c18a8ce59c7d655acb17532fa606dc2f449f2b83d7a`다.
프롬프트를 제출하거나 클립보드를 읽지 않았다. 이 초기 검사는 인증과 빈 Session
시작만 입증한다. 이후 스킬과 재개 검사는 아래에 기록하며 native read-only sandbox는
계속 사용할 수 없다. 일반 Grok suite는 `session/load` 응답 전에
같은 Session의 과거 update 1,025개를 즉시 버리고 이후 새 응답과 resumable outcome을
전달하는 경계를 별도로 검증한다. 다른 Session, 서버 요청, 응답 신원 오류와 기존
무관 메시지 대기열 상한은 계속 적용한다. 이는 결정적인 adapter 검사이며,
실제 문맥 재개 측정 결과는 아래에 기록한다.

최초 격리된 합성 스킬 프롬프트에서 Grok native skill watcher가 요청 없이 보내는
method 없는 `skills-reload` 유지보수 응답을 발견했다. Yo는 continuation anchor를
기록하기 전에 이 문자열 ID를 거절했다. 커밋 `f94d1a42`는 숫자 요청 ID 상관관계를
유지하면서 정확한 유지보수 acknowledgement 형식을 소비한다. Adapter 검사는 시작,
활성 프롬프트, 재개를 다루며 무관한 응답 거절과 프롬프트 조기 완료 방지도 포함한다.

수정된 바이너리는 2026-09-10 격리된 공식 Grok 실행에서 합성 model 제출 두 번과
관측된 watcher acknowledgement 두 번으로 검증을 통과했다. 첫 스킬 답변이 정확히
일치했고, 스킬 원본을 제거한 뒤 새 프로세스 재개에서도 정확한 회상을 검증했다.
continuation anchor 두 개와 journal prefix를 보존했다. 검사한 바이너리 SHA256은
`f72d343be2d2aecef3f844033a6e84fa59aa3ffc902c680ced0a6b5bc6200d6a`다.
소유한 tmux Session을 종료하고 임시 인증을 제거했다. 설치된 호스트는 이미지 prompt
지원을 광고하지 않으며 native read-only review sandbox는 계속 사용할 수 없다.

### Grok 사용자 질문 실제 서비스 흐름

2026-09-18에 커밋 `37a94b68`은 전용 140×44 Linux tmux Session에서 실제 사용자
질문 왕복을 완료했다. 정확한 Yo 실행은 `target/debug/yo --model host:grok`이었고,
일반 저장 상태와 캐시된 Grok login, 설치된 Grok `1.0.34 (3736acbc8658)`,
`grok-4.6`을 사용했다. 사전 account 관찰은 SuperGrok 주간
포함 한도가 82퍼센트 남았다고 보고했다. `grok models`는 `grok-4.6`과
`grok-4.5`만 광고했으므로 이 실행은 무료 Build model이 아니라 구독 포함 한도를
사용했다.

단일 prompt는 “Choose a validation color” 질문과 “Blue”, “Green” 선택지를 가진
`ask_user_question` 호출 하나를 요구했다. Grok은 실제 비공개 질문 request를 보냈고,
Yo는 `UserInputRequest` 하나를 게시했으며 TUI는 두 선택지를 표시했다. “Blue”를
선택하자 accepted 답안이 만들어졌고 Grok tool result는 정확한 질문과 답을 보고했으며,
assistant는 `Validation choice: Blue`로 완료했다. 저장된 Transcript에는 일괄 capture,
확정 답안 seal, 연결된 tool 완료, 최종 message, usage receipt와 완료 Turn이 순서대로
보존됐다. 영수증은 model call 2회, input token 37,073개, output token 173개,
cache-read token 19,200개, reasoning token 113개를 보고했다.

다른 tool 활동이나 repository 변경은 관찰되지 않았다. Ctrl+D로 shell에 돌아온 뒤
소유한 tmux Session을 제거했다. 이 실행은 설치 환경의 단일 선택 경로와 durable 결과를
입증한다. 일괄 이동, 직접 입력, 메모, 한도, 중단, 오래된 identity 거부와 미지원 다중
선택 동작은 계속 결정적 test가 기준이다. 이 실제 서비스 흐름은 macOS에서 반복하지
않았다.

### Mac v1 인터뷰 요청 흐름

2026-09-21 정확한 커밋 `9c3f6caaa5a00485f20078efde348ae564181aaa`가
저장된 Apple Silicon Mac의 `yo-tui` profile을 통과했다. Frame·크기 0 재진입,
all-target test와 Clippy `-D warnings`를 실행했다. 1,380바이트 증분 Git
번들을 검증하고 임시 checkout에서 빌드했다. 설치본, 일반 인증정보와 사용자 소유
tmux Session은 사용하지 않았다.

해당 커밋의 수정하지 않은 Fullscreen Yo 바이너리를 격리된 Mac tmux와
`HOME`에서 실행하고 작은
오프라인 ACP peer를 `host:grok`으로 연결했다. Peer가 비밀이 아닌
`_x.ai/ask_user_question`을 발행하자 Yo는 실제 `UserInputRequest` 활동으로
표시했다. 생성된 `yo.interview-draft/v1` 파일은 generation 1,
480바이트, mode 0600이었다. “Blue”를 선택해 accepted 답안을 돌려준 뒤
durable final seal이 초안을 삭제했다. 늦은 활동 이벤트가 더는 “a submitted
interview cannot become a new draft” 안내를 표시하거나 파일을 다시 만들지
않았다. 새 Session에서는 Esc로 요청을 중단했을 때 초안 하나가 남았고,
`/interview`는 `view`와 `discard`를 제공했다. `view`는 읽기 전용이었으며
명시적인 `discard`가 파일을 삭제했다.

이는 결정적 오프라인 peer를 사용한 실제 Mac Yo/ACP/TUI 요청 경로 검증이다.
공식 Grok 서비스나 유료 모델에 연락하지 않았으므로 실제 Provider 요청의
증거는 아니다. 위의 [Linux Grok 실제 서비스 흐름](#grok-사용자-질문-실제-서비스-흐름)은
별도 증거다.

### Mac TUI 검색과 외부 편집 검증

2026-09-22 저장된 Apple Silicon 호스트에 커밋
`0fd2086dff002ef6e5a6156edf13af8a3fd28ece`의 전체 이력 Git 번들
8,924,259바이트를 한 번 전송했다. SHA-256은
`679682042ffafd42782178a640d54830a491b26a8bfc67367573ab2490a90e56`였다.
임시 checkout에서 frame 검사 6개, 크기 0 재진입 검사 2개, `yo-tui` library
검사 1,028개와 추가 target 검사 4개, TUI Clippy `-D warnings`, 제품 CLI
빌드가 통과했다.

분리된 100×35 Mac tmux 소켓에서 수정하지 않은 오프라인 `chat_preview`
바이너리를 실행했다. `Ctrl+F`는 안내 문서의 “Charts” 제목을 결과에서
제외했고 완료된 Assistant 답변의 “Readable prose”를 1/1 결과로 찾았다.
Enter는 해당 과거 메시지로 이동하고 End는 최신 화면으로 복귀했다. Esc는
작성 중인 초안을 복원했다. `Ctrl+R`은 확정된 `markdown` 입력을 보여 주었고,
Esc는 기존 초안을 복원했으며 Enter는 전송 없이 해당 입력을 초안에 실었다.
`/find Readable`, `/status`, `/copy`도 예상 결과를 표시했다. `/copy`는
OSC 52 요청을 보냈다는 안내이며 터미널 클립보드의 실제 변경을 입증하지
않는다. 입력은 물리 Mac 키보드가 아닌 tmux 합성 키였고 모델 서비스 요청은
없었다.

프리뷰 실행 파일은 `Ctrl+G`가 외부 편집을 요청하면 정상 종료한다. 편집기
전환은 바깥쪽 `yo` CLI가 담당하기 때문이다. 이후 첫 Mac CLI test 빌드는
Linux 전용 `symlink` test import를 모든 Unix에서 가져와 엄격한 미사용 import
lint에 걸리는 문제를 발견했다. 로컬 소스와 바이트가 같은 조건부 import 수정을
임시 checkout에 적용한 뒤 외부 편집 focused 검사 4개, CLI library 검사
543개, CLI 통합 검사 5개, 추가 terminal 검사 2개와 CLI Clippy가 통과했다.
실제 `yo` 바이너리도 loopback Codex fixture를 쓴 격리 tmux에서 Fullscreen
검사 1개와 Inline 검사 6개를 통과했다. 여기에는 외부 편집 복귀, 편집 실패
복구, bracketed paste, 상태, 중단·재개와 정상 종료가 포함된다. 이 수정으로
Mac test 빌드 문제는 해결됐다.

같은 임시 checkout에서 격리된 Yo/Codex 홈과 Mac 내부 loopback Responses
fixture를 사용해 실제 `yo --fullscreen --model host:codex` 바이너리를 별도
tmux 세션에서 실행했다. 외부 모델 endpoint와 인증 정보는 사용하지 않았다.
fixture Turn이 완료되자 정해진 Assistant 답변과 사용량이 표시됐다. Mac
Ghostty에 접속한 사용자는 물리 `Ctrl+F` 검색, `Ctrl+G` 외부 편집, 여러 줄
Command-V 붙여넣기는 정상이라고 보고했으나 완료 소리는 듣지 못했다. 별도의
Mac tmux pane에서 완료된 fixture Turn의 원시 출력을 기록하니 BEL 바이트가
정확히 하나 있었다. `TERM=xterm-ghostty`를 설정한 별도의 Mac 가상 터미널
client도 test pane에서 BEL 하나를 받았다. Ghostty에서 직접 `printf '\a'`를
실행하면 벨 표시는 떴으나 소리는 나지 않았고 Terminal.app에서는 소리가 났다.
Ghostty의 유효 설정은 기본값
`bell-features = no-system,no-audio,attention,title,no-border`를 덮어쓰지 않았다. 따라서
소리가 나지 않은 원인은 Yo BEL 누락이 아닌 터미널 설정이다. Ghostty는
macOS 시스템 알림음용
[`bell-features = system`](https://ghostty.org/docs/config/reference#bell-features)을
문서화하며 사용자는 시각적 표시를 유지하기로 했다. 이어서 물리 한영 IME
조합·삭제·방향키 편집과 `/copy` 후 다른 Ghostty 창에서 Command-V로 답변을
붙여넣는 실제 클립보드 수신도 확인했다. 위의 recall·undo·status 검사는
합성 키로 확인했다.

### Grok 큰 문맥 재개

2026-09-11 같은 바이너리와 Grok `1.0.25 (f7e67d6988e2)`를 격리된 Linux 110×40
tmux에서 실행해 공식 서비스에 대한 합성 제출 일곱 번을 검증했다. 위임 Grok
suite는 64개 검사가 통과했고 환경 검사 두 개는 ignored였다. 앞서 통과한 CLI
build는 변경되지 않았다. 도구, 웹 검색, 하위 에이전트와 memory를 비활성화했고,
인증과 호스트 상태에는 임시 Grok home을 사용했다.

여섯 Turn에 합성 기록 960개, 총 입력 120,272바이트를 제공했고 각각 정확한 확인
답변을 받았다. 정상 종료한 뒤 새 Yo 프로세스에서 같은 Yo Session과 native Grok
Session을 재개해 앞부분·중간·마지막 batch의 무작위 checkpoint 값 세 개를 정확히
회상했다. 재개 prompt에는 checkpoint 이름만 주고 정답 값은 제공하지 않았다.
일곱 Turn이 모두 완료됐고 continuation anchor 일곱 개를 보존했으며, 기존 물리적
journal prefix 192,069바이트가 바이트 단위로 동일했다. 도구나 승인 실행은 없었다.

Native `session/load`는 상관관계가 일치하는 응답 전에 update 19개를 재생했고,
해당 응답 이후 일곱 번째 `session/prompt`가 전송됐다. 재생된 메시지는 Yo 답변을
중복 생성하지 않았다. 오프라인 감사는 정확한 답변, 입력 크기, Session·프로세스
신원, journal prefix, anchor, 순서와 cleanup을 확인했고 증거를 변조한 대조군
여섯 개를 거절했다. 계획한 제출 일곱 번을 재시도 없이 모두 사용했다. 검증 후
임시 인증, native 상태와 소유한 프로세스를 제거했다.

이는 측정한 입력 크기와 일곱 Turn 깊이에서의 재개를 입증한다. Update 1,025개의
mailbox 경계는 별도 결정적 검사이며, 이번 실행은 그만큼의 실제 update 재생,
context compaction이나 호스트 최대 context window에서의 동작을 입증하지 않는다.

## 로컬 tmux와 Linux SSH 검사

이전 사용자 관찰에서 SSH/tmux 터미널에 산 이미지가 표시되고 HTTP 링크가
열리는 것을 확인했다. 원격 README 파일 링크는 열리지 않았다. 해당 파일 링크
제한은 저장소 README에 기록되어 있으며 원격 파일 전송이나 내장 viewer는 없다.
이는 터미널·버전 matrix가 남아 있지 않은 사용자 보고이며 자동 픽셀 검사나 모든
터미널에 대한 주장은 아니다. 이전 로컬 메모의 “픽셀 확인 대기” 상태는 이 응답으로
해소됐다.

Linux 또는 macOS의 두 표시 mode에서 로컬 tmux를 검사한다.

```bash
cargo test -p yo-cli --test terminal_matrix local_tmux_ \
  -- --ignored --nocapture --test-threads=1
```

Linux의 두 표시 mode에서 SSH와 SSH 내부 tmux를 검사한다.

```bash
cargo test -p yo-cli --test terminal_matrix ssh:: \
  -- --ignored --nocapture --test-threads=1
```

로컬 tmux test는 지원하는 두 Unix host에서 실행할 수 있으며 호환되는
`tmux`와 Codex가 설치되어 있어야 한다. SSH test는 Linux 전용이다.
localhost에 격리된 `sshd`를 시작하고 임시 key를 생성한 뒤 fixture
디렉터리를 제거한다. 호환되는 로컬 `ssh`, `sshd`, `ssh-keygen`, Codex,
`USER`가 로컬 SSH account 이름으로 설정되어 있어야 한다. 중첩된 경우에는
tmux도 필요하다.

로컬 tmux는 임시 `HOME`, `CODEX_HOME`, XDG root, Yo 설정과 Session repository를
사용하는 interactive Bash 아래에서 `yo --model host:codex`를 실행한다. Codex
provider는 예약된 loopback listener만 가리키고 계정을 사용하지 않으며 인증 저장은
파일로 한정한다. 검사는 tmux의 foreground command 이름 대신 실제 입력 화면과
raw/no-echo mode를 기다린다. 빈 입력 `Ctrl+D` 뒤에는 셸이 상태 0으로 끝나기 전에
셸 terminal과 main screen이 복원되어야 한다. macOS에서만 일시적인 queued-input
`PENDIN` bit를 제외하고 나머지 전체 termios를 비교한다. Backend thread binding,
수락된 요청, 완료된 Turn과 loopback 연결은 모두 없어야 한다. 각 검사는 자신이
만든 tmux server, socket과 임시 Codex·Yo 상태를 제거한다.

모델 inference 없이 앱의 문자 입력과 terminal text paste를 모킹한다.

```bash
cargo test -p yo-cli --test terminal_matrix draft_input_and_bracketed_paste \
  -- --ignored --nocapture --test-threads=1
```

두 mode 모두 ASCII 입력과 Backspace, 완성형 한글의 cursor 이동과 Delete/Backspace,
한글과 emoji를 포함한 LF/CRLF paste를 받는다. Paste는 Turn 제출 없이 서로 다른
초안 행 세 개로 표시되어야 한다. `Ctrl+C`로 각 초안을 지우고 빈 입력 `Ctrl+D`로
종료, terminal 복원과 정리를 확인한다. Fixture는 전용 tmux buffer와
[`paste-buffer -p -r`](https://man.openbsd.org/tmux#paste-buffer)를 사용해 paste bracket을
요청하고 linefeed를 보존한다. 시스템 clipboard는 사용하지 않는다.
이는 해석된 문자와 terminal paste 검사다. 물리 key mapping, macOS IME의 조합 중
문자열·확정 과정과 terminal의 Command-V 단축키는
[직접 입력 검증](#현재-mac-직접-입력-검증)에서 별도로 확인했다.

외부 편집기 인계는 격리된 로컬 tmux 검사로 확인한다.

```bash
cargo test -p yo-cli --test terminal_matrix external_editor \
  -- --ignored --nocapture --test-threads=1
```

Fixture는 Yo를 시작하는 shell에 전용 `VISUAL` script를 설정한다. `Ctrl+G`가
Inline과 Fullscreen에서 편집된 여러 줄을 전송하지 않은 초안으로 되돌리는지,
편집기가 실패하면 원래 초안이 남는지, 전경 편집기의 `Ctrl+C`가 Yo를 종료하지
않고 `Ctrl+Z`가 멈춘 편집기를 취소해 복귀하는지 검사한다. 시작 직후 terminal을 읽는
편집기로 전경 인계 경쟁도 검사한다. 임시 초안 파일 삭제,
Yo 프로세스 신원 유지, 추론 요청 없음,
종료 후 shell terminal 복원도 검사한다.

무시된 `local_tmux_inline_status_shows_session_without_inference` 검사는 격리된
Inline tmux에서 `/status`를 열어 Session 식별자를 모델 요청 없이 표시하고
같은 shell 복원 경로로 종료하는지 확인한다.

2026-09-22에는 커밋 `5134078e`로 격리된 100×35 Linux Fullscreen tmux에서
새 프로세스 resume도 수동 확인했다. 현재 `yo` 바이너리로 완료된 기존 Session을
재개한 뒤 합성 `Ctrl+R`을 누르자 확정된 일반 prompt가 picker에 나타났다.
Enter 한 번으로 세 줄이 전송되지 않은 composer 초안에 복원되었고 두 번째 Enter는
누르지 않았다. `Ctrl+C`로 초안을 지운 뒤 빈 `Ctrl+D`로 종료하자 격리된 tmux
서버도 종료되었다. 전후 Session Journal cutoff는 122로 같아 새 제출은 확정되지
않았다. 재개한 binding은 `host:grok`/`grok-4.6`이었지만 이 검사는 모델 추론
요청을 보내지 않았다. 이는 Linux TUI 인계를 확인한 결과이며 실제 키 매핑이나
남은 Mac resume 검증의 증거는 아니다.

각 경로는 빈 입력 `Ctrl+D` 종료와 두 번 연속
`Ctrl+Z` → job 정지 → `fg` terminal generation을 모두 검사한다.
job-control 검사는 매 정지 구간의 터미널을 해당 경로의 실제 interactive shell
termios와 비교하고, `yo` 프로세스가 커널 stopped 상태인지 확인하며, 각 `fg`
뒤 요청한 표시 mode를 다시 획득하는지 확인한다. SSH 내부 tmux는 바깥 SSH
PTY 복구도 추가로 확인한다.

필요한 명령이나 assertion을 사용할 수 없으면 test는 실패한다. 빠진 환경을
성공한 skip으로 바꾸지 않는다.

### Current Codex Mac tmux verification

2026-09-12 수정 없이 `6aac838b` candidate tree
(`a7dfab60f1caea706c0fc9dbe02f50ba90d4fc64`)의 로컬 tmux 검사 여섯 개가 모두
통과했다. 환경은 설치된 Codex 0.154.0과 tmux 3.6a가 있는 macOS 26.6.2 arm64였다.

```bash
cargo test --locked -p yo-cli --test terminal_matrix local_tmux_ \
  -- --ignored --nocapture --test-threads=1
cargo clippy --locked -p yo-cli --test terminal_matrix -- -D warnings
```

Inline과 Fullscreen 각각 raw/no-echo input에 진입하고 빈 입력 `Ctrl+D`로 상태 0
종료, 셸 termios와 main screen 복원, 두 번의 stopped-job/`fg` generation을 완료했다.
입력 모킹 두 개도 ASCII와 완성형 한글 편집, 한글과 emoji를 포함한 서로 다른 LF/CRLF
paste 행 세 개, `Ctrl+C` 초안 지우기를 통과했다. Native Clippy도 통과했다.
Thread binding, 수락된 요청, 완료된 Turn과 loopback inference 연결은
모두 0이었다. 일반 Yo 상태와 읽기 전용 Mac source repository는 변하지 않았다.
Test 소유 tmux 자원과 임시 상태를 모두 제거하고, 이어 일회용 checkout, packet과
build/log 파일도 삭제했다. 실제 keyboard, IME의 조합 중 문자열·확정 과정과 terminal
Command-V는 [현재 직접 입력 검증](#현재-mac-직접-입력-검증)에서 확인했다.

## 현재 Mac 직접 입력 검증

2026-09-15에 설정된 arm64 Mac에서 승인된 `develop` 커밋 `35d3e47f`를 locked
dependency로 빌드했다. 전용 tmux pane에서 사용자는 물리 키보드의 한영 전환과
IME 조합, Backspace와 방향키 편집, terminal application의 실제 Command-V를
통한 한글·ASCII·emoji 두 줄 붙여넣기, `Ctrl+C` 초안 지우기를 수행했으며 입력
이상을 보고하지 않았다.

종료 후 관찰은 Yo 밖의 결과 wrapper 오류 두 건 때문에 반복했다. 첫 pane은
marker를 보존하지 않았고, 다음 zsh wrapper는 Yo가 반환된 뒤 shell의 읽기 전용
`status` 변수에 값을 할당했다. 결과를 보존하는 Bash 실행은 빈 입력의 `Ctrl+D`
종료 상태 0, alternate screen 해제와 shell termios의 정확한 복원을 기록했다.
검사한 binary SHA256은
`e25e97801ad161e930335ae669079913fd79870f4351149a3b4bc960bbc3860b`다.

Stock Fullscreen TUI는 offline Provider와 격리된 config, Codex, Session state를
사용했고 macOS native sandbox가 network access를 거부했다. Session repository의
backend binding, accepted request와 finished Turn은 모두 0건이었다. 이는 물리
terminal 입력과 종료 동작을 입증하지만 model service 동작은 입증하지 않는다.
전용 tmux session, 임시 checkout, build와 state는 제거했다. 설치된 Yo, 정상
credential, 기존 tmux session과 clipboard 설정은 유지했다.

## macOS 실제 host 증거

2026-07-30에 `develop` commit `085e763`으로 수용된 tree를 macOS 26.2
arm64에서 실행했다. 해당 host에서
`cargo test --workspace --all-targets`가 통과했다.

그다음 80x24 실제 zsh PTY에서 두 표시 mode를 실행했다. 두 mode 모두
raw/no-echo 입력에 진입하고 빈 입력 `Ctrl+D`로 정상 종료했으며,
`Ctrl+Z` → job 정지 → `fg` 세대를 두 번 완료했다. Fullscreen은 각
세대마다 alternate screen을 해제하고 다시 획득했으며, Inline은
alternate screen에 진입하지 않았다.

동일한 시나리오는 `-f /dev/null`과 격리된 socket을 사용한 tmux 3.6a에서도
통과했다. 매 정지 구간에서 shell termios가 복원됐고 각 `fg` 뒤 요청한
mode를 다시 획득했다. 이는 명시적인 실제 host 관찰이며 일반 cross-platform
test set에 포함된 검사가 아니다.

그다음 `develop` commit `af546a5`로 수용된 정확한 tree를 대상으로 80x24
zsh PTY에서 SSH 경로를 실행했다. SSH가 소유한 interactive zsh는 두 mode
모두 빈 입력 `Ctrl+D` 종료와 두 번의
`Ctrl+Z` → job 정지 → `fg` 세대를 완료했다. Inline은 alternate screen
밖에 머물렀고 Fullscreen은 매 세대마다 이를 해제하고 다시 획득했다. SSH
session 종료 뒤 로컬 PTY termios도 바뀌지 않았다.

동일한 SSH session 구조에서 `-f /dev/null`과 격리된 socket을 사용해 tmux
3.6a에도 접속했다. 매 정지 구간과 최종 종료 시점에 pane은 zsh로 돌아오고
alternate screen을 해제했으며 기준 termios와 일치했다. 각 `fg` 뒤에는
pane이 `yo`로 돌아오고 raw terminal 설정과 요청한 표시 mode를 다시
획득했다. 중첩 session 종료 뒤 바깥 로컬 PTY도 복원됐다. 이 SSH 관찰은
실제 원격 host를 사용했으며 일반 test set이 아니라 증거 기록이다.

### Apple Silicon 빌드와 주입 입력 검사 (2026-09-11)

2026-09-11 `f57e61e5` 기반 수정 트리를 macOS 26.6.2 arm64에서 고정된
`nightly-2026-05-22` toolchain으로 검사했다.

Native core suite는 706개 검사가 통과했다. CLI 검사는 unit test 524개와 integration
test 7개가 통과했고 환경 검사 18개는 ignored였다. `yo-core`,
`yo-backend-delegated-grok`, `yo-cli`의 Clippy, host-target Unix matrix와 native
오프라인 `chat_preview` 빌드도 통과했다. Linux에서는 core 706개, Grok 64개, CLI
unit test 532개와 integration test 7개가 통과했다. 변경한 workspace-reference
fixture는 별도의 집중 검사 31개를 통과했다. 이 수치는 중복되므로 독립적인 검사
범위인 것처럼 합산하지 않는다.

사용자가 Mac tmux pane에 오프라인 실행 파일을 열고 해당 창의 자동 입력을
명시적으로 요청했다. 주입한 입력으로 한글 텍스트, 커서 이동, Backspace와 삽입,
bracketed 여러 줄 붙여넣기, 초안 지우기, 빈 입력의 `Ctrl+D` 종료, 이후 셸 명령과
canonical 입력·echo 복구를 확인했다. 사용자 소유 tmux pane은 셸에 남겨 두었다.
Native preview 바이너리 SHA256은
`34ebe6cf4633d15e36d826ff8ce9774ec63aa2a7014027a35fa753cd6eb04a85`다.
모델 요청이나 계정 접근은 없었다. 이는 실제 Mac tmux 경로에서 입력을 주입한
검사이며, 물리 키보드·IME 조합·터미널 앱의 Command-V는
[직접 입력 검증](#현재-mac-직접-입력-검증)에서 별도로 확인했다.

임시 체크아웃, 소스 전송 파일과 테스트 프로세스를 정리했다. 사용자의 tmux Session과
기존 클립보드 설치는 보존했다.

## Mac 클립보드에서 Linux yo로

2026-09-10에 candidate `8343292f0281b9e8d7321fd504bed97ead205b7c`의 이미지
첨부를 macOS 26.6.2 arm64에서 SSH Unix 소켓 전달을 거쳐 격리된 tmux의 실제
Linux Rust 바이너리까지 검증했다. Mac은 `pngpaste` 0.2.3과 candidate의
`tools/clipboard_bridge.py`를 사용했으며 Homebrew 경로를 명시해야 했다.
설치한 helper의 해시는
`5e3fa05d25b1d856a89927dc1e5cd236d9801b80a4c6ac42af6704ace0b57c58`였다.

Mac의 기존 pasteboard 항목을 읽어 Mac 메모리에만 보관하고 일반 pasteboard에
64 × 32 합성 PNG를 넣었다. 임시 guard는 실제 `pngpaste` 출력을 Mac 안에서
버퍼링하고 fixture 픽셀과 변경되지 않은 pasteboard revision을 확인한 뒤에만
바이트를 전달했다. `tmux send-keys`로 전달한 Ctrl+V가 20·40·96열에서 이미지를
첨부했다. 기존 pasteboard를 복원한 뒤 guard가 다음 캡처를 거절했을 때도 yo는
기존 이미지와 초안을 모두 유지했다. 모델 Turn은 제출하지 않았다. 앱은 정상 종료했고
검증이 만든 SSH 연결, helper, 소켓, tmux와 fixture를 정리했다.

연결 실행기는 별도로 Linux 실제 PTY에서 Ctrl+Z → foreground 복귀 두 번과
빈 입력창의 Ctrl+D 종료를 검증했으며 셸 터미널 설정도 복원됐다. 이 관찰은 native
획득, 전달과 Rust 입력 처리를 확인한다. 실제 Mac 키보드 단축키나 터미널의 픽셀
이미지 프로토콜 검증은 포함하지 않는다. 클립보드 원본은 여전히 프로세스마다 명시적으로
선택해야 한다.

이후 정식 SSH 설정 경로도 같은 Mac fixture, 첨부 폭, 붙여넣기 실패 시 보존과
정상 종료 검사를 Python 실행기·bridge 서비스·전달 소켓 없이 통과했다.
해당 Rust 바이너리의 SHA256은
`f124a62a87ab04c06a163fa9c6752859eb4171cd0debaeaf5fbde07f3b8739cc`였다.
테스트 전용 SSH shim이 guard가 있는 native reader를 선택했으며 실제 제품 worker,
SSH 인증과 내장 원격 supervisor가 획득을 수행했다. 기존 클립보드를 복원하고 임시
fixture를 제거했으며 모델 요청은 없었다. Python supervisor의 별도 합성 검사는
시간 초과·연결 종료 신호에서 reader와 자식 정리를 확인하고 실패한 캡처의 부분 출력을
거절한다.

## 플랫폼 검사 범위

현재 실행 가능한 환경 매트릭스의 범위는 다음과 같다.

| host와 경로 | Inline | Fullscreen | 증거 |
|---|---:|---:|---|
| Linux 직접 실제 PTY | Yes | Yes | 일반 `yo-cli` test가 두 mode의 종료·반복 일시정지/재개와 Fullscreen termination을 검사 |
| Linux 로컬 tmux | Yes | Yes | ignored test가 정상 종료와 두 번의 shell 기반 일시정지/재개를 검사 |
| Linux SSH | Yes | Yes | ignored test가 정상 종료와 두 번의 원격 shell 기반 일시정지/재개를 검사 |
| Linux SSH 내부 tmux | Yes | Yes | ignored test가 정상 종료, 두 번의 중첩 일시정지/재개, 바깥 PTY 복구를 검사 |
| macOS compile | — | — | 실제 macOS 26.2 arm64 host에서 workspace all-target test 통과 |
| macOS 직접 실제 PTY | Yes | Yes | 실제 host에서 정상 종료와 두 번의 shell 기반 일시정지/재개를 검사 |
| macOS 로컬 tmux | Yes | Yes | ignored test가 정상 종료와 두 번의 shell 기반 일시정지/재개를 검사하며, 실제 host 실행으로 mode 재획득과 shell termios 복원도 확인 |
| macOS SSH | Yes | Yes | 실제 host에서 정상 종료, 두 번의 원격 shell 기반 일시정지/재개, mode 재획득, 바깥 PTY 복원을 검사 |
| macOS SSH 내부 tmux | Yes | Yes | 실제 host에서 정상 종료, 두 번의 중첩 일시정지/재개, pane mode·termios 전환, 바깥 PTY 복원을 검사 |

`tools/validation/yo-cli-unix-matrix.sh`는 현재 Unix host의 모든 `yo-cli`
target을 검사한다. 출력은 해당 실행만 설명한다. 현재 host는 verified이고
다른 host는 `unverified(not run on current host)`이다. 이는 다른 host를
사용할 수 없다는 뜻이 아니며, 별도로 기록된 실제 host 증거를 지우지도
않는다. CI workflow는 Linux와 macOS에서 동일한 compile 검사를 독립적으로
실행한다. compile은 터미널 동작 증거를 대신하지 않는다.

## 매트릭스 실행 결과 보고하기

결과는 작지만 명확하게 기록한다.

```text
Host:
Route and mode:
Command:
Result: passed | failed | unverified
Observed failure or missing prerequisite:
```

한 경로의 결과로 다른 경로를 추정하지 않는다. 로컬 tmux가 통과했다고 해서
같은 host의 SSH나 SSH 내부 tmux까지 verified로 표시하지 않는다.

계약:
[rendering 검증 기준](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.surface.validation-matrix.md)

실행 결과를 passed, failed, unverified로 분류하려면
[검증](./#결과-읽기)으로 돌아간다.
