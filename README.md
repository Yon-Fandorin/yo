# yo

`yo`는 Rust 기반 코드 에이전트 인터페이스다.

현재 첫 목표는 macOS와 Linux의 최신 terminal, tmux, SSH, 원격 tmux에서
동작하는 agentic TUI 기반과 이를 사람과 에이전트가 함께 이해할 수 있는
Developer Docs를 구축하는 것이다.

기본 `yo` 호출은 대화형 TUI를 연다. 셸에서 응답 하나만 필요하면 같은
Session·Backend 경로를 `-p` 또는 `--print`로 실행할 수 있다.
시작 안내와 상태줄에는 내장 연결의 프로바이더·모델 또는 위임 호스트·보고된 모델을
함께 표시한다. 모델을 보고받지 못한 호스트는 `model unreported`로 표시하며,
모델 전환 뒤에도 실행 주체를 유지한다.

`/new`는 현재 연결을 사용해 새 대화를 만든다. Codex와 내장 연결은 현재 모델을 고정하며,
모델을 보고하지 않는 위임 호스트는 기존 호스트 설정을 사용한다. 기존 대화가 저장되어 있고
진행 중 작업이나 대기 입력이 없을 때 사용할 수 있다. 새 대화 준비에 실패하면 기존
대화를 유지하고 이유를 표시한다.

`/resume`은 현재 작업 폴더의 저장된 대화 목록을 열고, `/resume UUID`는 해당 대화를
직접 재개한다. 저장된 연결·모델·작업 폴더를 사용하며 재개할 수 없는 항목은 이유와
`yo session UUID` 기록 조회 방법을 표시한다. 재개 준비에 실패하면 현재 대화를 유지한다.

`/fork`는 진행 중 작업이나 대기 입력이 없을 때 현재 저장 경계의 문맥을 새 대화로
복사한다. 현재 내장 managed 연결의 exact replay를 지원하며 저장된 모델·도구·작업 폴더를
유지한다. 새 대화가 준비된 뒤 선택을 바꾸고, 실패하면 기존 대화를 유지한다.
상속한 대화는 원래 세션과 경계를 표시하며 읽을 수 있다. 부모의 승인 요청이나 사용량은
새 대화의 실행 상태에 포함하지 않는다.
Codex·Grok 같은 위임 호스트는 정확한 분기 경계를 검증하는 기능이 없어 현재 거절한다.

`/fork at`은 현재 대화의 저장된 과거 지점을 선택해 분기한다. 당시 입력과 모델을
목록에서 확인하고 Enter로 선택한다. 선택한 지점까지의 문맥만 새 대화로 복사하며,
목록을 연 뒤 대화 상태가 바뀌면 새 목록을 열도록 안내한다. Esc로 취소하거나 준비가
실패하면 현재 대화와 작성 중 입력을 유지한다. 조회 한도를 넘는 저장 기록은 일부만
검증해 실행하지 않고 거절하며, 검증된 지점이 많아 목록만 줄인 경우에는 이를 표시한다.

`/tree`는 현재 작업 폴더의 저장된 대화와 분기 관계를 보여준다. 검증된 저장 기록만
계보로 사용하고, 부모가 없거나 관계를 확인할 수 없으면 그 상태를 표시한다.
재개 가능한 항목을 선택하면 해당 대화를 연다. 조회 한도에 도달하면 일부만 표시됐음을
안내하며, 트리를 열기만 해서는 모델을 호출하거나 새 대화를 만들지 않는다.
위아래 키로 항목을 살펴보고 PgUp/PgDown으로 긴 설명을 읽는다. 현재 대화나 재개할 수
없는 항목도 읽을 수 있지만 Enter로 재개하지는 않는다.

Codex 호스트에서는 `$`로 선택한 스킬을 제출할 때 최신 목록과 파일 내용을 검증한다.
스킬 지침은 해당 입력에 고정되어 저장되며, 이후 파일이 바뀌어도 복구 시 다시 읽지 않는다.
내장 모델과 로컬 Grok 호스트는 아래 `skills.roots`의 명시적 로컬 스킬 경로를 사용한다.
스킬 지침은 모델과 분리된 입력 기능이며 실행 가능한 도구나 sandbox 권한을 늘리지 않는다.

대화형 화면에서 `/preview`를 입력하면 오프라인 UI 테스트 공간이 열린다.
임의의 메시지로 스트리밍 응답을 확인하거나 `tools`, `long-tools`, `error`, `long`, `markdown`, `tables`, `diff`, `approval`, `interview`로
모의 도구 출력·실패 복구·긴 본문·Markdown·표·변경 내용·승인 선택·두 단계 인터뷰를 확인한다. `/preview` 또는 `/exit`로
원래 대화에 돌아온다. 프리뷰 입력은 모델에 전송되지 않고 실제 대화에 저장되지 않는다.
실제 작업이 진행 중이면 먼저 완료하거나 중단해야 한다.
첫 안내 문서는 코드·파일 검색·도구·승인·상태·차트 예시를 분류해 보여준다.
`Alt+Up`으로 안내 시작에 이동하고 위아래 키로 스크롤할 수 있다.
승인은 ↑/↓로 선택하고 Enter로 확정하며 Esc로 거절한다. 인터뷰는 번호나 직접 쓴 답변을
Enter로 제출하고 Esc로 중단한다. 승인은 요청 범위를 확인한 뒤 `Approve request`로
제출하며 결과가 채팅 기록에 남는다. `approval-scopes`는 세션 승인·영구 명령 규칙·
네트워크 허용/거부 선택지를 모의한다. 모든 선택은 오프라인 프리뷰 안에만 남는다.
실제 Codex 승인은 서버의 선택지와 호환 기본 규칙을 따르며, `Decline and stop`은 현재 턴도 중단한다. 프리뷰 승인은 실제 명령을 실행하지 않는다.

`footnotes` 프리뷰는 Markdown 각주를 확인한다. `[^이름]` 참조와 정의를 같은 이름으로
표시하고 정의 안의 링크·코드·목록을 렌더링한다. 미정의 참조와 원문 내보내기는 보존한다.

모델 연결 없이 별도 터미널에서 확인하려면 `cargo run -p yo-tui --example chat_preview`를
실행한 뒤 `/preview`를 입력한다. 테스트 실행기의 시간 경고가 UI에 끼어들지 않는다.

`/preview` 안에서 `showcase`는 구문 강조·차트·이미지·대체 표시를 한 번에 보여준다.
`syntax`, `charts`, `images`, `media-errors`로 개별 확인도 가능하다.
`image /절대/경로.png`는 명시한 로컬 PNG/JPEG(1 MiB 이하)를 읽어 프리뷰 안에 표시한다.
파일은 수정하지 않으며 외부 URL은 자동으로 가져오지 않는다.

코드 fence의 언어에 따라 키워드·문자열·주석·숫자·함수를 구분한다. 미지원 언어와
`linechart`, `stepchart`, `scatterchart`는 `계열 이름: 값들` 형식으로 최대 4개 계열을
공통 축에 표시한다. 번호가 붙은 점과 범례로 계열을 구분하며, 같은 격자 점에 겹치는
계열은 `×`로 표시하고, 점이 다른 혼합 셀은 선 모양을 유지한다. 좁은 폭에서도 계열 이름과 원래 수치를 유지한다.
`/preview chart-series`에서 확인할 수 있다.

큰 코드 블록은 원문을 유지한다. `chart` fence의 `항목: 숫자`는 막대 차트로,
`sparkline` fence의 공백으로 구분한 숫자는 추세로 표시하며 원래 수치도 함께 남긴다.
아주 작은 유한값도 0과 구분하며, 지수 표기가 긴 차트도 축 정렬과 입력 수치 표기를 유지한다.
Markdown의 PNG/JPEG data URI 이미지는 대화 안에서 셀 기반 컬러 미리보기로 표시한다.
일반 파일·웹 이미지 링크는 참조로 남는다. 이미지 미리보기는 최대 64열·20행으로
제한하며, 단색·ASCII에서는 명암 문자로 바뀐다. `linechart` fence는 숫자를 연결한
선 차트를 축·범위와 함께 표시한다. 넓은 막대 차트는 이름·막대·수치를 열에 맞춘다.
`histogram` fence는 공백으로 구분한 숫자의 분포를 구간별 개수로 표시한다. 첫 줄의
`bins: 4`로 구간 수를 1~32개로 지정한다(기본 8개, 표본 최대 64개).
구간은 `[하한, 상한)`이며 마지막 구간은 최댓값을 포함한다. 상수 데이터나 수치 정밀도
한계에서는 구간 수를 줄인다. 원래 수치·구간·개수를 함께 남기고 기존 차트 색상을 사용한다.
`/preview`에서 `histogram`으로 확인할 수 있다.
`scatterchart` fence는 `4,12 8,18 16,17`처럼 공백으로 구분한 X,Y 좌표를 산점도로
표시한다. 두 축의 범위와 입력 좌표를 함께 보존하며 점 사이를 연결하지 않는다.
좁은 폭에서는 좌표 목록으로 표시하고 `/preview`의 `scatter`로 확인할 수 있다.
선·계단·산점도 fence의 첫 줄에 `height: 3`처럼 적으면 그래프 높이를 2~16행으로
조정한다(기본 6행). 축·범위·입력 수치 표시는 별도이며 `/preview`의 `chart-heights`로
요약용 그래프와 상세 그래프를 비교할 수 있다.

Fullscreen에서 확인 가능한 Kitty 호환 터미널은 원본 PNG를 그래픽 프로토콜로 전송한다.
JPEG는 EXIF 회전·반전을 적용한 뒤 원래 해상도로 PNG 변환 후 전송한다.
셀 미리보기도 같은 방향을 사용하며 작은 이미지는 원본보다 확대하지 않는다.
`/preview`의 `image-orientation`에서 회전 정보가 있는 JPEG를 확인할 수 있다. 화면에 이미지 전체가 들어올 때만 원본을
표시하며, 잘리거나 overlay가 겹치면 셀 표시로 전환한다. Inline 모드는 셀 표시를 유지한다.
`tui.colors.code_text: terminal`처럼 코드 글자색을 바꿔도 이미지 색상·픽셀 전송은 유지한다.
이미지의 색상 능력은 터미널과 선택한 테마가 결정한다. 단색 테마는 기존 명암 문자를 사용한다.
`YO_TUI_IMAGE_PROTOCOL=cells`는 원본 전송을 끈다. tmux에서는 외부 지원을 추측하지 않는다.
지원 터미널과 `allow-passthrough on`을 확인한 경우 `YO_TUI_IMAGE_PROTOCOL=kitty-tmux`
환경변수로 실행할 수 있다. 이 경로의 실제 터미널 픽셀 검증은 별도로 필요하다.
iTerm2·Sixel 전송은 지원하지 않는다.

입력창에서 `Ctrl+V`를 누르면 클립보드 이미지를 준비해 커서 위치에 미리보기와 `[image]`를 넣는다.
설명을 작성하고 Enter를 눌러야 모델에 전송된다. 준비 실패나 취소는 기존 초안을 보존한다.
터미널의 일반 텍스트 붙여넣기는 그대로 텍스트 입력으로 처리한다.
직접 실행하는 Linux 데스크톱은 `wl-clipboard` 또는 `xclip`, Mac은 `pngpaste`를 사용한다.
SSH·tmux에서 Mac 이미지를 받으려면 `config.yaml`에 클립보드 SSH 원본을 지정한다.
yo가 Ctrl+V 때 직접 연결하고 종료하므로 별도 실행기나 상시 터널은 필요하지 않다.
설정 방법은 [클립보드와 SSH 전달](docs/ko/src/architecture/runtime-flow.md#클립보드-획득과-ssh-전달)에 있다.

`/attach /절대/경로.png`는 yo 실행 호스트에 저장된 PNG/JPEG를 고르는 보조 명령이다.
입력창의 마지막 줄에 쓰고 Enter를 누르면 준비만 하며, 입력 전체를 다시 제출해야 전송된다.
파일당 4 MiB, 입력당 원본 합계 8 MiB·16개까지 허용한다. 이미지는 방향 정보를 적용해
최대 한 변 2048픽셀·약 209만 픽셀의 RGBA8 PNG로 정규화한다. 원본 파일은 변경하지 않는다.
첨부 준비와 모델 지원 확인에 실패하면 입력을 유지하며, 큐·다시 편집·재개에는 같은 PNG를 보존한다.

관리형 Kimi Code 연결은 검토된 이미지 프로필을 명시한 경우 이미지 전송을 지원한다.
해당 `connections.yaml` binding의 `profile`에 `image_input_profile: kimi-code-png-advisory/v1`을
설정하며, 모델·주소·thinking·replay·토큰 한도가 해당 프로필과 모두 맞아야 연결을 허용한다.
기존 연결에는 자동 추가하지 않는다. 지원이 확인되지 않은 다른 백엔드·모델은 전송 전에 거절한다.
OpenRouter의 NVIDIA Nemotron 3 Nano Omni 무료 모델도 명시적 이미지 연결로 사용할 수 있다.
[연결 정의와 사용법](docs/ko/src/workflows/provider-catalogs/openrouter.md#명시적-무료-이미지-연결)에
따라 추가한다. 이미지가 없는 요청에도 무료 경로 제한을 유지하며 자동 재시도하지 않는다.
터미널의 픽셀 표시 지원은 모델의 이미지 입력 지원과 별개다.
SSH에서 `/attach`의 경로는 yo를 실행하는 원격 호스트에 있어야 한다.
이미지 연결의 컨텍스트 수치는 추정값과 요청당 여유분을 구분해 표시하며, 실제 사용량으로 간주하지 않는다.

모델 답변은 제목, 강조, 목록, 인용, 링크, 코드 블록을 구분해 표시한다.
일반 문장은 단어 단위로 줄바꿈하고 코드의 공백과 기호는 보존한다.
일반 텍스트(`text`·`txt`·`plaintext`) 패널은 반복 언어 라벨을 생략한다.
코드 블록은 장식 선 없이 전용 배경·언어 헤더·안쪽 여백으로 구분하고,
좁은 폭에서는 여백을 줄여 본문을 자동 줄바꿈한다. 표는 열 안에서 줄바꿈하고 아주 좁은 폭에서는 `열 이름: 값`으로 표시한다. `diff`·`patch` 코드 블록은
추가·삭제 행은 빈 오른쪽 영역과 줄바꿈 행까지 옅은 초록·빨강 배경으로 표시하고,
구간 정보와 `+`·`-` 기호도 유지한다. 밝은 테마는 파스텔 배경, 흑백은 색 없이 경계와 부호를 사용한다.
사용자 입력과 도구 로그는 원문 그대로 표시하며, 저장된 대화와 일반 텍스트 출력에는
Markdown 원문이 유지된다. 도구 출력은 진행·완료·중단·실패 상태를 제목으로
구분하고, 로그 본문과 실제 실패 사유를 서로 다른 스타일로 표시한다.

Codex host에서는 실행 시작부터 명령과 작업 디렉터리를 표시하고, 완료 시 실제 출력·종료
코드·소요 시간을 반영한다. 파일 변경은 경로·이동 경로·diff 본문을 보존하며, 실제
`FileChange` 이벤트도 추가·삭제 배경과 행 수를 갖춘 변경 블록으로 표시한다. 긴 변경
블록은 `Ctrl+O`로 펼친다. MCP 도구는 서버·도구 이름, 인수, 결과와 오류를 표시한다.
도구 로그의 Markdown은 해석하지 않는다. 사용량은 검증된 영수증을 사람이 읽을 수 있는
수치로 표시하며, 완료된 최신 관측값을 하단에 남긴다. 하단의 `Last`는 세션 누적 사용량이나
잔여 한도가 아니다. 완료된 사용량은 기본 접힘 화면에서 입력·출력만 짧게 표시하고
`Ctrl+O`로 캐시·추론·컨텍스트 상세를 펼친다. 개별 관측과 원문·집계는 보존한다. `/preview`의 `changes`, `usage`, `mcp`로 각 표시를 확인할 수 있다.

내장 명령은 모델용 결과와 별도로 더 긴 출력을 보존한다. 채팅의 **Retained output · /output**
안내가 있으면 `/output`에서 모델용 결과에 없는 중간 내용도 읽을 수 있다. 보존분 자체가
잘렸을 때는 뷰어에 `Partial output`이 표시된다. 보존 원문도 자격증명 검사와 기존 세션
저장 정책을 따르며 무제한 보존은 아니다. `/preview shell-retained`로 120행 예시를 확인한다.

`/changes`는 대화에서 관찰한 파일 변경을 읽는 전용 화면이다. 좌우 키로 파일을 바꾸고
위아래·휠로 diff 전체를 읽으며 F1로 대화에 돌아온다. Git 작업 트리 전체를 스캔하거나
파일을 수정하는 명령은 아니다. 상단에는 현재 파일 경로가 고정되며, 좁은 폭에서는
생략 표시와 경로 끝부분을 남긴다. 전체 경로와 diff 원문은 보존한다. 추가·삭제 파일의 원문도 올바른 `+`/`-` 변경 줄로 표시한다.
Codex와 Grok ACP의 계획 갱신은 완료·진행·대기 단계로 표시한다. Grok 계획은 전체
목록을 교체하고 우선순위도 보존하며, 도구 실행이 끼어도 같은 계획을 갱신한다.
Grok 도구 결과는 인자·콘텐츠·원시 결과를 공통 도구 렌더러에 전달하며, 부분 갱신에서
생략한 필드와 명시적으로 비운 결과를 구분한다.
파일 링크는 실행 호스트의 `file:///...` 주소를 외부 터미널에 전달한다.
SSH로 접속한 경우 서버의 경로이며, 웹 링크가 열려도 해당 파일이 열리는 것은 아니다.
현재 yo에는 클릭한 원격 파일의 전송이나 내장 파일 뷰어 연결이 없다.
파일 열기는 외부 터미널의 파일 처리 기능에 의존한다.

호스트는 `LinkResolver`로 확인한 파일을 Markdown 링크에 연결할 수 있다. 파일 확인은
렌더링 전에 수행하며, 일반 `file:` 주소나 임의 경로를 자동으로 활성화하지 않는다.
`chat_preview --custom-links`에서 `/preview`를 연 뒤 `file-links`를 입력하면 실제 저장소의
README를 연결한 예시를 볼 수 있다. 클릭 지원은 외부 터미널에 따라 다르다.

호스트는 `TuiStatusLine`과 `AgentPoll::StatusLine`으로 확장 상태를 별도 푸터 행에 표시할 수 있다.
상태는 키 순서로 정렬하며 전체 snapshot으로 갱신·삭제한다. 대화·저널에는 넣지 않고,
좁은 폭에서는 생략 표시를 사용한다. 기존 `muted` 색상 설정이 적용된다.
`/preview`에서 `status`, `status-update`, `status-clear`로 모델 호출 없이 확인할 수 있다.

Codex의 사용자 질문은 순서대로 제시한다. 두 번째 질문부터 `Shift+Tab`으로 이전 질문에
돌아가 수정할 수 있으며, 현재 초안과 선택·메모를 복원한다. 최종 질문을 제출할 때 수정된
전체 답변을 전송한다. 이동 지원은 공통 프로필로 전달하며 프로바이더 어댑터가 결정한다.
선택 번호 또는 직접 입력한 답은 원래 질문 ID에 묶어 한 번만 전달한다. 선택지에서 Tab을
누르면 추가 메모를 작성하고 Enter로 함께 보낸다. 메모 화면의 Tab은 선택지로 돌아가며
초안을 보존한다. 이 기능을 지원하는 호스트에서만 표시한다. 제출한 답과 메모는 채팅에
완료 표시로 남으며, 후속 질문 대기와 마지막 응답 전송을 구분한다. 인터뷰가 중단되면
기록된 답변 수와 남은 질문을 요약한다. Esc는 Turn을 중단한다. 비밀 입력은 현재 지원하지 않는다. `/preview`의 `plan`, `interview`와
`changes` 입력 후 `/changes`로 모델 호출 없이 확인할 수 있다.

긴 도구 로그는 채팅에서 앞부분과 최신 출력을 우선 보여주며 `Ctrl+O`로 전체 표시를
전환한다. 실패 사유는 접지 않고 저장·텍스트 출력과 터미널 scrollback에는 전체 내용이
유지된다. 긴 입력은 입력창 안에서 스크롤되며 아래 구분선에 보이는 행 범위를 표시한다.
실행 중 다음 요청은 `Alt+Enter` 또는 `Alt+Q`로 예약한다. 완료 후 하나씩 새 작업으로
전송하며, 실패·중단·입력 거절 시에는 예약을 일시정지한다. 하단에 예약 개수와 정지 상태를
표시한다. `Alt+R`은 예약을 멈추고 빈 입력창으로 다음 메시지를 꺼낸다. 수정해서 일반 Enter로
보내거나 삭제할 수 있다. 입력창을 비우고 `Alt+Enter`/`Alt+Q`를 누르면 남은 예약을 재개한다.
예약은 세션 메모리에 최대 16개·총 64 KiB까지 보관하고 종료하면 사라진다. 명령은 예약하지
않는다. 선택한 참조는 신원과 함께 보관하며 실제 전송할 때 다시 검증한다.
Alt+Enter를 개행으로 설정한 환경은 Alt+Q를 사용한다.

`@` 후보에서 고른 파일·디렉터리는 입력의 구조화된 참조로 전달한다. 실행 호스트가
전송 직전에 작업 공간·경로·종류·접근 권한을 다시 확인한다. 내용이나 ignore 설정만
바뀌면 사용할 수 있지만 삭제·심볼릭 링크 치환·루트 변경·접근 거절은 초안을 보존하고
거절한다. 선택 시 파일 내용을 읽거나 디렉터리를 재귀 첨부하지 않는다. 스킬 참조는 아직
실행 admission이 없어 제출 시 명시적으로 거절하며, 예약도 실제 전송 시 같은 검증을 따른다.

입력 편집에서 `Ctrl+A`·`Ctrl+E`는 실제 줄의 시작·끝으로 이동하고, `Ctrl+U`·`Ctrl+K`는
커서 앞·뒤를 줄 경계까지 삭제한다. 줄 경계에서 다시 삭제하면 이웃 줄을 연결한다.
`Ctrl+←/→`는 공백으로 구분된 단어의 시작·끝으로 이동하고, `Ctrl+W`는 커서 앞의
공백과 단어를 삭제한다. 한글·결합 문자·이모지는 글자 중간에서 나뉘지 않는다.
`Ctrl+Y`는 최근 연속 삭제 내용을 복원하며, 질문 메모에서도 같은 키를 사용할 수 있다.
과거 대화로 이동하면 위치와 `End`로 최신 메시지에 돌아가는 안내가 나타난다.
전체화면에서는 휠·트랙패드로 대화를 스크롤한다. tmux는 `mouse on` 설정이 필요하며,
앱 종료 시 마우스 캡처를 해제한다. tmux copy mode의 스크롤백과 앱의 대화 기록은 별개다.

대화형 테마는 `yo --theme default|light|mono`로 선택한다. `default`는 기존
청록색 강조와 짙은 요청 배경, `light`는 밝은 터미널용 강조색과 옅은 요청 배경,
`mono`는 터미널 기본색을 사용한다. 본문 배경색은 터미널 설정을 따른다.
`--ascii`와 함께 쓸 수 있고 `/preview`에도 적용된다. 옵션은 현재 실행에서만
설정 파일보다 우선하며 `--print`나 하위 명령과 함께 사용하지 않는다.
테마를 저장하려면 기존 `config.yaml`에 다음 설정을 추가한다.
설정 파일 위치는 [설정 안내](docs/src/architecture/runtime-flow.md)에서 확인한다.

```yaml
tui:
  theme: light
```

`theme`를 생략하면 `default`가 적용된다. 실행 중 설정 파일을 다시 읽지는 않는다.

```bash
yo -p "이 저장소의 테스트 명령을 알려줘"
printf '이 오류를 분석해줘\n' | yo --print
printf '참고 문맥\n' | yo -p "이어서 요약해줘"
printf '이 finding을 다시 확인해줘\n' | yo -p --resume SESSION_ID
yo -p --model host:codex --sandbox read-only "이 diff를 검토해줘"
yo -p --model host:grok --sandbox read-only "이 diff를 검토해줘"
```

마지막 예처럼 stdin과 위치 prompt를 함께 주면 stdin 뒤에 prompt가 이어진다.
성공 시 stdout에는 완료된 최종 답변과 마지막 줄바꿈만 기록되고, 진행 상태·도구
활동·사용량·Session 식별자는 섞이지 않는다. 실패 진단은 stderr로 가며 종료 코드는
0이 아니다. `--model TARGET`과 새 Session 전용 `--no-tools`도 print mode에서
각각 독립적으로 사용할 수 있다. `--resume SESSION_ID`는 저장된 동일 Session의
Provider·Account·Model, 도구, replay 상태를 그대로 사용해 Turn 하나를 잇는다.
따라서 `--model`, `--no-tools`, `--continue` 및 terminal 표시 옵션과 함께 쓸 수
없고, 복구가 실패해도 새 Session이나 다른 모델로 대체하지 않는다. `-p/--print`는
top-level 하위 명령과 한 호출에 섞을 수 없으며, 하위 명령과 같은 한 단어를 prompt로
쓰려면 `yo -p -- session`처럼 `--` 뒤에 명시한다.

`--sandbox read-only`는 새 print-mode Codex·Grok host Session만을 위한 제한
프로필이다. 로컬 작업공간 쓰기, 웹 검색, 네트워크, 권한 상승을 닫고 Grok에는 읽기
도구만 노출한다. 제한 프로필은 Session binding에 저장되므로 후속 Turn은
`yo -p --resume SESSION_ID`로 이어가며 flag를 반복하지 않는다. native model,
대화형 실행, `--no-tools`, 새 Session이 아닌 resume과의 조합은 시작 전에 거절된다.

저장된 Session의 토큰·캐시 사용량은 `yo usage SESSION_ID`로 확인한다. 계정 자체의
현재 한도는 별도 개념이다. `yo account`는 모든 지원 계정의 마지막 관측값을 보여주고,
`yo account kimi`처럼 Provider만 지정하면 그 Provider의 모든 계정을,
`yo account kimi:ACCOUNT`처럼 지정하면 한 계정을 보여준다. `--refresh`를 붙인 경우에만
선택한 범위를 다시 조회하며, 각 결과에는 마지막 갱신 시각이 함께 표시된다. 조회
결과가 하나면 상세 화면을, 여러 결과면 컬럼형 요약 표를 기본으로 사용하며 `--detail`로
언제든 상세 화면을 강제할 수 있다. 여러 결과가 터미널 폭에 들어가지 않으면 상세 화면으로
전환한다. 여러 계정 요약은 `PROVIDER`, `ACCOUNT`, `PLAN`,
`LIMITS`, `UPDATED` 컬럼 표이며, `LIMITS`에는 작은 수직 level meter가 함께 표시된다.
`--ascii`와 `--format`은 공통 출력 옵션이다. `--ascii`는 지원되는 text meter glyph를
ASCII로 바꾸고, `--format json`은 현재 account에서 지원한다. 아직 JSON을 지원하지 않는
명령에서 해당 format을 사용하면 실행 전에 명확한 미지원 오류를 낸다.
캐시가 없는 delegated host는 `Local Codex` 또는
`Local Grok`의 `Account  Not resolved` 행으로 표시된다. 어느 경로도 새 Agent Session이나 모델 요청을
만들지 않는다. Yo는 Provider가 보고한 플랜과 한도만 공용 화면으로 표시하며, 유효한
잔여량 관측이 없는 경로에서는 이를 합성하지 않는다. Agent가 읽을 때는 `--format json`을
덧붙인다. 캐시가 없는 delegated host의 실제 계정은 `yo account PROVIDER --refresh`로
host에 질의하면 확정된다. 구현이 참고한
upstream 소스와 정확한 어댑터 경계는
[`Account capacity`](docs/src/workflows/account-capacity.md)에 기록한다.

코드를 탐색하고 검증하는 방법은
[`Developer Docs`](docs/src/README.md)에서 시작한다. 저장소 작업 방식은
[`CONTRIBUTING.md`](CONTRIBUTING.md)를 따른다.


출력 레이아웃은 `config.yaml`의 `tui`에서 조정한다. 테마 변경과 터미널 재진입 후에도 유지된다.

```yaml
tui:
  theme: default
  max_body_width: 96
  tool_head_rows: 4
  shell_tail_rows: 5
  diff_head_rows: 8
  show_images: true
  show_reasoning: true
  show_diagrams: true
  image_max_width: 48
  code_padding: 1
```

`max_body_width`는 본문 최대 열 수이며 생략하면 터미널 폭을 사용한다. 1열은 한글 표시를 위해 2열로 조정한다. 접힌 도구·diff는
각각 지정한 앞부분과 마지막 세 행을 남긴다. 앞부분 설정의 기본값은 2·6이며, 짧은 출력과
실패 사유는 접지 않는다. `Ctrl+O`와 `/changes`로 전체 내용을 읽을 수 있다.
Rust 호스트는 `TuiSession::with_output_preferences(OutputPreferences::default()...)`로 같은
설정을 전달할 수 있다. 원본 record는 변경하지 않는다.
`show_images: false`는 이미지 디코딩과 픽셀 표시를 생략하고 대체 설명을 남긴다.
`image_max_width`는 이미지 표시 최대 열 수이며 기본값과 상한은 64, 최솟값은 1이다.
64를 넘는 값은 64로 제한한다. 표시 폭을 줄여도 원본 이미지 데이터는 유지한다.
`code_padding`은 코드·diff 패널의 좌우 여백이며 기본 1칸, 0~8칸이다.
8을 넘는 값은 8로 제한하고 좁은 화면에서는 본문 공간을 남기도록 줄인다.
Rust API의 `with_code_padding(u16)` 또는 프리뷰 실행 옵션 `--code-padding=N`으로도 조절한다.
Rust API에서는 `with_images(bool)`과 `with_image_max_width(NonZeroU16)`로 설정한다.

내장 모델과 로컬 Grok에서 쓸 스킬은 `config.yaml`에 검색 경로를 지정한다.
경로를 생략하면 로컬 스킬을 자동 검색하지 않는다. Codex는 자체 스킬 목록을 계속 사용한다.

```yaml
skills:
  roots:
    - path: .agents/skills
      scope: workspace
    - path: /absolute/path/to/my-skills
      scope: user
```

상대 경로는 현재 대화의 작업 폴더 기준이다. 저장 대화를 재개하면 기록된 작업 폴더를
기준으로 해석한다. `..`가 포함된 경로 대신 상위 폴더의 절대 경로를 지정한다.
각 경로의 바로 아래 폴더에 `SKILL.md`를 둔다.
예를 들어 `.agents/skills/review/SKILL.md`에 다음 내용을 저장한다.

```markdown
---
name: review
description: 변경 사항의 오류와 누락된 검증을 점검한다.
user-invocable: true
---
변경 사항을 검토하고 근거가 되는 파일 위치를 보고한다.
```

`name`과 `description`은 생략할 수 있다. `enabled` 또는 `user-invocable`이 `false`이면
목록에 비활성 사유를 표시하고 선택하지 못하게 한다. 두 값의 기본값은 `true`다.
동일 이름도 경로·출처별로 구분한다. `$` 목록에서 선택한 뒤 제출할 때 본문과 정책을
다시 검증하며, 선택 후 파일이 바뀌면 재선택을 요청한다. 본문 전체를 입력에 고정하므로
이후 파일 삭제나 경로 설정 변경은 저장된 지침의 복구에 영향을 주지 않는다.

최대 16개 경로를 설정하며 파일당 256 KiB 상한을 적용한다. 심볼릭 링크와 일반 파일이
아닌 항목은 실행 대상으로 허용하지 않는다. 본문의 참조 파일·스크립트는 미리 실행하거나
재귀적으로 읽지 않는다. 필요하면 에이전트가 기존 도구·승인 경계를 통해 접근한다.

내장 managed 모델에 실행 도구를 추가하려면 `config.yaml`의 `tools.commands`에
고정 실행 파일과 입력 스키마를 등록한다. 다음 예제는 작업 폴더의 스크립트를 Python으로
실행하며, 스크립트는 표준 입력으로 JSON 객체 한 줄을 받고 결과를 표준 출력에 쓴다.
`executable`은 실제 환경의 절대 경로로 바꾼다.

```yaml
tools:
  commands:
    - id: project-check
      name: project_check
      description: Check the selected project target
      executable: /usr/bin/python3
      script: tools/project_check.py
      argv: []
      parameters:
        type: object
        properties:
          target:
            type: string
        required: [target]
        additionalProperties: false
```

호출마다 승인이 필요하다. 모델 입력은 셸 문자열이나 명령 인자로 삽입하지 않고 JSON으로
전달한다. `executable_args`는 스크립트 앞, `argv`는 뒤에 고정 인자로 전달한다.
세션 시작 때 파일과 실행 구성을 고정하고, 승인 후 실행 직전에 다시 확인한다.
구성이나 파일이 바뀌면 기존 세션의 재개·분기·모델 전환을 거절하므로 변경된 도구는 새
세션에서 사용한다. `--no-tools`와 기존 도구 구성이 저장된 세션은 도구를 자동 추가하지
않으며, 위임 호스트의 도구 구성은 해당 호스트가 관리한다.

반복해서 쓰는 프롬프트는 같은 `config.yaml`의 최상위 `prompts`에 문자열로 저장한다.
설정 후 yo를 다시 시작하면 `/prompt`로 이름을 확인하고 `/prompt review`로 본문을
편집기에 불러온다. 내용을 확인·수정한 뒤 Enter로 전송한다. 로딩 자체는 제출하지 않는다.

```yaml
prompts:
  review: |-
    변경 사항의 오류와 빠진 검증을 찾아줘.
    근거가 되는 파일 위치도 함께 알려줘.
```

이름은 영문·숫자·`_`·`-`로 이루어진 1~64바이트이며 최대 128개를 설정한다.
본문은 비어 있지 않은 UTF-8 문자열로 최대 64 KiB이다. 공백·탭·CR·LF를 그대로
보존하며 다른 제어문자는 거부한다. 설정 파일 전체의 기존 1 MiB 상한도 적용된다.
본문의 `$name`, `@path`, 변수나 셸 문법은 문자 그대로 보관한다. 파일 include나
명령 실행을 하지 않으며 skill·workspace reference를 자동으로 만들지 않는다.

Codex 검색 이벤트는 검색어·페이지 URL·찾는 문자열을 원문 패널로 표시한다.
action 상세가 비어 있어도 항목에 검색어가 있으면 표시하고, 원본 query/action은
`ToolRenderInput.output.arguments`로 커스터마이징할 수 있다. `/preview search`로 확인한다.
추론 이벤트는 호스트가 제공한 공개 요약을 생성 중에도 표시한다. 문단별 갱신을 순서대로
조합하고 완료 요약으로 교체한다. `/preview reasoning`으로 확인할 수 있다.


의미별 색상을 직접 지정하려면 같은 `tui` 아래 `colors`를 추가한다. 값은 따옴표로 감싼
`"#RRGGBB"` 또는 `terminal`이며, 생략한 항목은 선택한 기본 테마를 따른다.

```yaml
tui:
  theme: default
  colors:
    accent: "#ac9ce0"
    user_background: "#262333"
    code_background: "#1d1d29"
    code_header_background: "#2d2a3f"
    diff_added_background: "#1c302b"
    diff_removed_background: "#39222e"
```

사용 가능한 항목은 `accent`, `text`, `muted`, `user_text`, `user_background`, `code_text`,
`code_background`, `code_header_background`, `diff_added`, `diff_removed`,
`diff_added_background`, `diff_removed_background`, `success`, `warning`, `error`,
`syntax_keyword`, `syntax_string`, `syntax_comment`, `syntax_number`, `syntax_function`,
`syntax_type`, `reasoning_text`, `chart`, `chart_2`, `chart_3`, `chart_4`이다. 차트 선·막대·축의 `chart`는
생략하면 accent를 따르고 지정하면 제목 색상과 독립적으로 적용된다. 공개 추론 본문·숨김 안내의 `reasoning_text`는
기본적으로 `muted`를 따르며 별도로 지정하면 우선한다. 제한된 색상 터미널에서는 256색으로 변환하고, `mono`나 색상 미지원에서는
기본색을 사용한다. 굵기·밑줄 같은 강조는 유지된다. 설정 오타와 잘못된 색상은 오류로 알린다.

Rust 호스트는 `ThemeOverrides::with_color`와 `TuiSession::with_theme_overrides`를 사용한다.
`cargo run -p yo-tui --example chat_preview -- --custom-colors`로 사용자 색상 예시를 볼 수 있다.

MCP·동적 도구 결과의 텍스트는 JSON 이스케이프 대신 실제 줄바꿈으로 표시한다.
텍스트 리소스는 URI·본문·메타데이터를 구분하고, 구조화된 결과와 미지원 블록도 보존한다.
`/preview mcp`에서 예시를 확인하며 `Ctrl+O`로 접힌 결과를 펼친다.
MCP `resource_link`는 이름·제목·주소·설명·형식·보고된 크기를 구분한다.
추가 메타데이터와 원문은 보존하며 주소를 열지는 않는다. `/preview resource-link`로
확인할 수 있고 기존 테마·접기·사용자 렌더러가 적용된다.

내장 `resource`도 MIME 형식과 안팎의 메타데이터를 표시한다. 텍스트의 코드 하이라이팅을
유지하며 PNG/JPEG blob은 기존 이미지 표시·폭 설정을 따른다. `/preview embedded-resource`는
코드·메타데이터·이미지를 함께 보여주는 오프라인 예시다.

MCP PNG/JPEG와 동적 도구의 내장 이미지 결과는 원본 데이터로 표시한다.
`/preview mcp-image`로 실제 PNG가 담긴 구조화된 도구 출력 예시를 확인한다.
Codex의 이미지 생성 이벤트도 같은 공통 도구 출력으로 표시하며, 생성 PNG와 진행·실패 정보, 저장 경로 메타데이터를 보존한다.

Rust 호스트는 `TuiSession::with_tool_renderer(Some(ToolRenderer::new(...)))`로
도구 본문을 Markdown으로 가공할 수 있다. 콜백 입력은 `kind`(ToolCall/ToolResult),
`source`(원문 본문), `outcome`(관측된 종료 결과), `expanded`(펼침 요청 상태),
`columns`(설정 적용 후 폭)이며 `None`을 반환하면 원문을 표시한다.
`expanded`로 접힌 요약과 펼친 상세를 다르게 구성할 수 있으며 `Ctrl+O` 전환은 캐시를 갱신한다.
코드·표·차트·이미지는 기존 렌더러와 색상·이미지 표시·접기 설정을 따른다.
상태 제목·실패 footer·원문 내보내기는 콜백 결과로 교체하지 않는다.
콜백은 반복 호출될 수 있으므로 I/O 없이 결정적으로 동작해야 하며 동작 변경 시 새
`ToolRenderer`를 설치한다. `with_tool_renderer(None)`은 기본 표시로 복원한다.
`input.output`이 있으면 검증된 `ToolOutput`의 도구·서버·원본 인수·결과 객체를 사용할 수 있다.
`source`는 읽기 쉬운 adapter 본문이며, 구조화된 출력을 제공하지 않는 adapter도 지원한다.
`outcome`은 `TranscriptActivityOutcome`의 완료·실패·중단 또는 아직 종료 관측이 없는
`None`이다. 본문 문자열로 상태를 추측하지 않는다. `mcp-failure` 프리뷰로 일부 결과와 실패를 확인한다.
오프라인 Rust 예시는 `--custom-tools`로 `docs.search` 프리뷰 본문을 꾸민다.

일반 모델 답변은 `AssistantRenderer`와 `TuiSession::with_assistant_renderer`로 표시를 바꿀 수 있다.
콜백은 원본 본문·현재 폭·finalized 여부를 받고 Markdown 또는 `None`을 반환한다.
실패·중단 문구는 별도로 보존하며 사용자 입력·도구·승인·문서에는 적용하지 않는다.
256 KiB를 넘거나 표시할 수 없는 결과는 원래 답변으로 복귀한다. I/O 없이 결정적으로
동작해야 하며 새 핸들로 교체하면 캐시가 갱신된다. 원문 내보내기는 영향을 받지 않는다.
Rust 프리뷰를 `--custom-answers`로 실행하고 `markdown` 사례에서 확인한다.

호스트가 보낸 `ActivityDocument` 본문은 `DocumentRenderer`와
`TuiSession::with_document_renderer`로 커스터마이징한다. 콜백은 원본 문서, 관측된 종료 상태,
펼침 여부와 폭을 받고 Markdown 또는 `None`을 반환한다. 제목·실패 footer·원문 내보내기는
유지하며 도구·승인·추론 요약에는 적용하지 않는다. 측정 중 재호출될 수 있으므로 I/O 없이
결정적으로 동작해야 한다. 핸들을 교체하면 캐시가 갱신된다. 큰 결과나 레이아웃 실패는
원래 문서로 복귀한다. 독립 Rust 예시의 `--custom-documents` 옵션을 켜고
`/preview terminal-wait`와 `Ctrl+O`로 요약·상세 표시를 확인한다.

Grok ACP 승인도 공통 선택 패널에서 일회성·기억되는 허용 및 거절 범위를 표시한다.
지원하지 않는 종류는 비활성화하며, 같은 턴의 정확한 미종료 파일 호출은 `/changes` 검토에 연결한다.
기억되는 정책의 실제 저장 범위는 에이전트가 관리한다.

`ToolOutput`은 정확한 `yo.tool-output/v1` 스키마를 가진 텍스트 snapshot으로 전달한다.
원본 인수·결과·미디어는 기존 journal의 message segment와 seal로 보존하며 저장 형식은 바꾸지 않는다.
기본 렌더러는 인수를 JSON 패널로, 도구 텍스트를 리터럴 코드 패널로 표시한다.
본문의 Markdown을 이미지나 링크로 추측하지 않으며, 명시적인 이미지 블록만 이미지 경로로 보낸다.
명시적인 `type: "diff"` 콘텐츠는 공통 diff 색상과 줄바꿈을 사용하며 `source`로 원본을 보존할 수 있다.
`/preview tool-diff`는 실제 Rust 화면에서 확인하는 오프라인 예시다.
지원하지 않는 스키마는 일반 텍스트로 남는다. 이미지 URL을 자동으로 가져오거나 모델 입력으로 첨부하지 않는다.

Codex의 `willRetry: true` 오류는 경고색의 재시도 안내로 즉시 표시한다.
일시 오류를 이후 최종 실패 사유로 재사용하지 않고 완료된 턴의 뒤늦은 재시도 알림은 무시한다.
`/preview retry`는 이 안내의 오프라인 예시다. 실제 이벤트에 없는 재시도 횟수·대기 시간은 표시하지 않는다.
서버·설정 경고는 활성 턴 없이도 채팅에 표시한다. 세션 경고는 일시적인 화면 안내이며
journal에 저장하지 않는다. `/preview warning`으로 확인할 수 있고 경고 색상은 `warning` 역할을 따른다. 프리뷰 안에서 `deprecation`과
`approval-warning`을 입력하면 사용 중단 안내와 승인 검토 경고도 확인할 수 있다.
`approval-diff`는 승인 요청에 연결된 파일을 `/changes`로 검토하는 예시다.
Codex의 `contextCompaction` 항목은 압축 진행·완료 안내로 표시하고 `accent` 색상을 따른다.
`/preview compaction`에서 전환을 확인할 수 있다. 호스트가 보내지 않은 요약·토큰 수는 만들지 않는다.
호스트가 `ActivitySummary`로 제공하는 압축·분기 요약은 Markdown으로 표시하고 `Ctrl+O`로 펼친다.
`tool_head_rows`가 요약의 접힌 행 수도 조절한다. `/preview summary`, `/preview branch`는
이 출력 형식의 오프라인 예시다. 현재 Codex 압축 항목에는 요약 본문이 없어 자동으로 생성하지 않는다.
Codex 공개 추론 요약도 Markdown과 접기를 지원한다. `tui.show_reasoning: false`는 본문만 숨기며
다른 안내와 실패 사유, 저장 원문·내보내기는 유지한다. Rust API는 `OutputPreferences::with_reasoning`이다.


Mermaid 코드 블록은 터미널 다이어그램으로 표시한다. `tui.show_diagrams: false` 또는
`OutputPreferences::with_diagrams(false)`로 원본 코드 표시를 선택할 수 있다.
`/preview diagrams`에 흐름도·시퀀스 예시가 있다. 본문 폭에 맞지 않거나 지원하지 않는
문법·크기 제한에 걸리면 원문을 표시하며, 코드 색상·배경 설정을 따른다.
Gantt와 style/class/click/linkStyle 지시문은 현재 원문으로 표시한다.


작업 계획은 완료·진행·대기 상태와 실제 완료 수를 표시한다. 진행 단계는 `accent`, 완료 단계는
`success`, 대기 단계는 `muted` 색상을 사용하며 긴 설명은 단계 본문에 맞춰 줄바꿈한다.
`/preview plan`으로 확인할 수 있다. 턴 종료나 표시 완료가 단계 상태를 자동으로 바꾸지는 않는다.


제안 계획은 `ActivityDocument`의 제목과 Markdown 본문으로 표시한다. 스트리밍 조각을 합치고
최종 본문으로 교체하며, 창 폭 변경·접기·중단 후에도 원문을 보존한다.
`/preview proposed-plan`에서 초안에서 최종 계획으로 바뀌는 화면을 확인할 수 있다.

백그라운드 터미널의 대기·입력 알림은 프로세스 ID, 알려진 명령, 입력 원문을 코드 블록으로
표시한다. 기존 코드 색상·본문 폭·접기 설정을 공유한다. 오프라인 프리뷰의
`terminal-wait`와 `terminal-input`은 표시만 재현하며 프로세스에 입력하지 않는다.

턴 종료 시 Codex가 `durationMs`를 제공하면 결과와 밀리초 정밀도의 소요 시간을 표시한다.
보고되지 않은 시간은 추정하지 않는다. `/preview turn-duration`으로 예시를 확인할 수 있다.

구조화된 `read`/`read_file`/`readFile` 결과는 `path` 또는 `file_path`의 확장자로
코드 색상을 선택한다. 파일 URI 리소스에도 적용하며 오류·알 수 없는 확장자는 일반
텍스트로 표시한다. 기존 `tui.colors.syntax_*`와 코드 배경 설정, 사용자 `ToolRenderer`를
그대로 사용한다. `/preview file-read`은 파일을 읽지 않는 오프라인 예시다.

실제 managed 백엔드도 도구 완료 시 검증된 인수·결과를 구조화된 출력으로 전달한다.
파일 읽기 코드 색상과 사용자 렌더러를 적용하면서 모델에 전달하는 결과 원문은 유지한다.

쓰기 도구의 문자열 `content`는 **Proposed file content** 코드 패널로 표시한다.
인수만 준비된 호출은 **Tool call prepared**로 구분하고, 실제 결과·오류는 별도로 유지한다.
`/preview file-write`에서 파일을 쓰지 않고 확인할 수 있다.

`read_files` 결과는 파일별 코드·줄 범위·다음 읽기 위치·오류로 표시한다.
잘리거나 형식이 맞지 않는 결과는 원문을 유지한다. `/preview files-read`로 확인할 수 있다.

파일 수정 인수는 **Proposed replacements** diff로 표시하고, 실제 결과가 보고한 교체 수는
따로 표시한다. `/preview file-edit`은 파일을 바꾸지 않는 예시다.

명령 도구는 실행 문자열을 코드 패널로 표시한다. 내장 `run_command`의 완전한 결과는
종료 상태·stdout·stderr를 구분하고, 모호하거나 잘린 결과는 원문을 유지한다.
`/preview shell`은 명령을 실행하지 않는 예시다.

파일 목록은 파일·디렉터리 구분, 반환 항목 수, 부분 목록 여부를 표시한다. 실제 백엔드의
잘림 정보를 사용하며 파일 이름을 Markdown으로 해석하지 않는다. `/preview files-list`로
디렉터리를 읽지 않고 예시를 확인할 수 있다.

`find` 도구는 검색 패턴·결과 경로·보고된 결과 제한을 구분한다. 알 수 없는 메타데이터는
JSON 원문을 유지한다. 기존 테마·접기·사용자 렌더러가 적용되며 `/preview files-find`는
실제 파일을 검색하지 않는 예시다.

`grep` 내용 검색은 패턴·검색 조건·결과와 일치 개수 제한·행 잘림 안내를 구분한다.
`/preview content-search`에서 실행 없이 확인할 수 있다. 검색 결과의 파일명이나 내용을
재해석하지 않으며 기존 테마·접기·사용자 렌더러를 그대로 사용한다.

Codex 명령 실행도 명령 코드 블록과 합산 출력, 종료 코드·시간으로 표시한다. 스트리밍은
구조화된 스냅샷으로 누적하며 최종 출력으로 교체한다. `/preview codex-shell`로
실행 없이 표시를 확인할 수 있다. 사용자 지정 도구 렌더러에는 구조화된 원본이 전달된다.

셸 출력은 기본적으로 마지막 5개 표시 행을 남기고 명령·종료 정보를 유지한다.
`tui.shell_tail_rows`로 행 수를 조절하고, `0`이면 일반 도구 접기를 사용한다.
`Ctrl+O`는 전체 출력을 펼친다. `/preview shell-tail`로 긴 출력 예시를 확인할 수 있다.

셸 결과의 `details.fullOutputPath`와 잘림 메타데이터가 보고되면 전체 출력 경로와
줄·바이트 수, 제한, 부분 줄 여부를 표시한다. `/preview shell-truncated`는 파일을
만들지 않는 예시다. 경로 표시는 파일을 열거나 내려받지 않는다.

내장 `run_command`는 실행 중 stdout·stderr를 구분해 갱신한다. 갱신은 최대 초당 10회로
묶으며 종료·취소 뒤에는 진행 출력을 닫는다. 별도 진행 출력 승인 정책을 통과한 내용만
표시하고 모델에는 최종 결과만 전달한다. `/preview shell-progress`는 실행 없이 진행·완료
표시를 확인하는 예시다. 이미 잘렸거나 표시 예산을 넘은 진행 출력은 최종 결과까지 보류한다.

접힌 셸 출력은 전체 로그의 셀 배열을 만들지 않고 마지막 표시 행만 보관한다.
65,535행을 넘는 로그나 좁은 화면에서 매우 길게 개행되는 한 줄도 최신 내용을 표시한다.
전체 펼치기·텍스트 내보내기의 기존 좌표 한도는 아직 별도 제약으로 남아 있다.


`/output`은 보관된 도구 출력을 페이지 단위로 읽는 화면이다. 마지막 도구부터 열고
위·아래, PageUp/PageDown, Home/End와 마우스 휠로 이동한다. 좌우 키로 도구를 바꾸고
F1으로 Chat에 돌아온다. End는 갱신되는 출력의 끝을 따라가며 위로 이동하면 해제된다.
65,535행을 넘는 원문도 탐색하며 `tui.max_body_width`와 테마의 도구 본문 색상을 따른다.
구조화 결과는 어댑터가 보관한 `plain_text`를 표시한다. 화면에 표시할 수 없는 문자는
escape 표기로 대체한다. 보고된 외부 출력 파일을 읽는 기능은 아직 포함하지 않는다.


Markdown의 HTTP·HTTPS 링크와 본문·표에 적힌 일반 웹 주소는 OSC 8을 지원하는
터미널에서 클릭할 수 있다. 문장 끝 부호는 제외하고 주소 내부의 괄호는 유지한다.
주소는 보이는 텍스트와 별도로 보관하므로 한글·표의 개행·화면 폭 변경에도 유지된다.
`tui.hyperlinks: false` 또는 Rust API `OutputPreferences::with_hyperlinks(false)`로
OSC 8 출력을 끌 수 있으며 글자·색상·원문은 그대로 남는다. 터미널 자체의 URL 자동 인식은
별개다. 코드 블록·상대 경로·file URL·실행 스킴은 링크화하지 않는다.
`/preview links`는 브라우저를 열지 않고 링크와 원문 대체 표시를 확인하는 예시다.

채팅에서 `Alt+↑/↓`는 현재 폭으로 배치된 메시지·도구 항목의 시작으로 이동한다.
항목 중간에서 위로 이동하면 그 항목의 시작으로 돌아가며, 마지막 항목을 지나 아래로
이동하면 최신 출력 추적을 재개한다. 프롬프트 초안은 유지한다.

`Alt+O`는 현재 읽는 활동 항목만 접거나 펼친다. 위로 스크롤한 상태에서는 화면의
첫 항목, 최신 출력을 따라가는 상태에서는 마지막 항목이 대상이다. `Alt+↑/↓`로
항목 시작을 찾아갈 수 있다. `Ctrl+O`는 개별 설정을 지우고 전체 펼침 상태를 바꾼다.
커스텀 도구·문서 렌더러도 항목별 `expanded` 값을 받는다.

호스트는 `TuiDocument::new(ActivityDocument { title, markdown })`로 검증한 문서를
`AgentPoll::Document`로 보낼 수 있다. 모델 Turn이나 저널 record 없이 표시하며,
기존 문서 렌더러·테마·개별 펼침과 화면 원문 내보내기를 사용한다. 같은 문서를 다시
보내면 별도 항목으로 추가되므로 중복 방지는 호스트가 맡는다. 프로세스 재시작 시 자동
복원하지 않으며 `/preview`의 `session-document`로 표·코드 안내 예시를 확인한다.

세션 안내를 처음부터 펼쳐 보이려면 검증한 `TuiDocument`에 `.with_expanded(true)`를
적용한다. `false`는 접힘으로 시작하고, 생략하면 현재 전체 펼침 상태를 따른다.
이후 사용자의 `Alt+O`·`Ctrl+O` 조작은 계속 적용된다. `session-document` 프리뷰는
이 옵션으로 표와 코드를 처음부터 보여준다.

실제 `/help`는 명령 목록과 키보드 안내를 펼쳐진 문서로 보여준다. 명령 목록은 등록된
정의에서 생성하며 항목 탐색·접기·입력 편집·승인·인터뷰 조작을 함께 설명한다.
모델에 제출하지 않고 대기 중인 요청에도 응답하지 않는다. 기존 문서 테마·렌더러를
적용하며 좁은 폭에서는 스크롤로 처음과 끝을 읽을 수 있다.
