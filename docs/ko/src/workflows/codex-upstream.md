# Codex app-server upstream 따라가기

설치된 Codex minor line이 거부되거나 upstream schema 또는 event가 바뀌었을
때, 혹은 adapter를 의도적으로 새 app-server로 옮길 때 이 흐름을 사용한다.
이는 운영 검증 가이드이며 adapter 계약의 두 번째 소유자가 아니다.

정확히 허용한 minor line은 compatibility check와 같은 위치인
[`protocol.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/codex/protocol.rs)가
소유한다. 동작 경계는 계속
[Codex app-server KnowledgeUnit](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.backend.codex-app-server.md)이
소유한다. 현재 version 목록을 이 가이드에 복사하지 않는다.

## 허용 전 gate

새 실행 파일이 설치됐다는 사실은 compatibility 증거가 아니라 후보라는
뜻이다. 알 수 없는 minor line은 적용 가능한 gate가 모두 통과할 때까지
fail-closed로 유지한다.

| gate | 확인하는 것 | 확인하지 못하는 것 |
|---|---|---|
| 공식 문서와 release 확인 | 문서화된 lifecycle과 발표된 변경 | yo의 비공개 adapter와의 compatibility |
| version별 schema 확인 | 후보의 정확한 wire 형태 | runtime 순서나 인증된 동작 |
| 결정론적 adapter test | parsing, correlation, mapping, failure 동작 | 설치된 process와의 compatibility |
| 설치본 initialize test | 실제 stdio handshake와 cleanup | 완료된 coding Turn |
| 설치본 coding-loop test | 실제 Turn, tool, file-change, event, cleanup 흐름 | 다른 host나 terminal 경로 |
| TUI smoke test | 사용자가 진입하고 제출하고 관찰하고 종료할 수 있음 | 해당 환경에서 실행하지 않은 macOS, SSH, 중첩 tmux |

이 gate를 느슨한 minimum-version 비교로 대체하지 않는다. 이전에 검증한
line은 제거에 대한 별도 compatibility 증거와 review가 없으면 유지한다.

## 후보 확인하기

후보 실행 파일을 기록하고
[공식 app-server 문서](https://developers.openai.com/codex/app-server)와
[공식 Codex release](https://github.com/openai/codex/releases)를 읽는다.
app-server 문서는 생성된 schema가 이를 만든 Codex version에 종속된다고
명시한다.

후보 schema는 저장소 밖에서 생성한다.

```bash
schema_dir="$(mktemp -d)"
codex --version
codex app-server generate-json-schema --out "$schema_dir"
```

yo가 사용하는 wire surface만 비교한다.

- `initialize`와 `initialized`
- `thread/start`, `turn/start`, `turn/steer`, `turn/interrupt`
- Turn과 Item lifecycle notification
- text, command-execution, file-change update
- approval server request와 response
- process shutdown과 transport closure

field가 추가됐다고 항상 안전한 것은 아니며, 관련 없는 큰 schema diff가
항상 blocker인 것도 아니다. 관련 있는 각 차이를 `backend/codex`, 결정론적
test, 이 adapter가 만드는 provider 중립 event까지 따라간다. 별도 Slice에서
tracked schema의 필요성을 입증하지 않았다면 생성된 schema는 일회성 증거로
유지한다.

## 계층별 증거 실행하기

먼저 후보 minor line을 허용하지 않은 상태에서 결정론적 fixture를
갱신하거나 추가한다. allowlist를 바꾸기 전에 기대 흐름과 이를 구분하는
failure를 모두 검사한다.

```bash
cargo test -p yo-core backend::codex::tests
cargo test -p yo-core backend::codex::protocol::tests
```

그다음 후보가 설치된 환경에서 실제 initialization 경계를 실행한다.

```bash
cargo test -p yo-core \
  backend::codex::tests::local_codex_initializes_and_shuts_down \
  -- --ignored --nocapture
```

인증과 model 사용 권한이 있으면 일회용 workspace에서 전체 coding loop를
검증한다.

```bash
cargo test -p yo-core \
  agent_session::tests::codex::local_codex_completes_a_real_file_change \
  -- --ignored --nocapture
```

마지막으로 `yo-cli`를 build하고 적용 가능한 terminal 경로에서 TUI smoke
run을 한 번 수행한다. 응답만 요구하는 prompt를 제출하고, 완료된 응답을
관찰한 뒤, 빈 prompt에서 종료한다. compatibility 변경이 terminal 경로에
영향을 준다면 로컬 shell 결과로 추정하지 말고 해당
[terminal matrix](../validation/terminal-matrix.md) 명령을 실행한다.

## line 허용 또는 거부하기

관련 증거가 통과한 뒤에만 minor line을 허용한다.

1. compatibility set에 정확한 minor line을 추가한다.
2. 이전 검증 line을 의도적으로 폐기하는 경우가 아니면 유지한다.
3. positive test가 허용한 모든 line을 실행하게 한다.
4. negative test는 실제로 검증하지 않은 line을 사용한다.
5. 잘못되거나 알 수 없는 version은 대응 가능한 오류와 함께 fail-closed로
   유지한다.
6. [Slice 종료 기준선](../validation/#slice-종료-기준선)을 실행한다.
7. provider compatibility는 제품 경계의 failure 동작을 바꾸므로
   fresh-context review를 받는다.

gate가 실패하면 version 범위를 넓히지 않는다. 처음 바뀐 wire 소유자를
찾아 비공개 adapter와 결정론적 증거를 갱신한 뒤 실제 경계를 다시 실행한다.
사용할 수 없는 인증, host, terminal 경로는 passed가 아니라 unverified로
기록한다.

## 검증 결과 보고하기

승인 commit 또는 review packet은 간결하고 재현 가능하게 유지한다.

```text
Candidate Codex version:
Official docs or release inspected:
Relevant schema differences:
Deterministic commands and results:
Installed initialize result:
Installed coding-loop result:
TUI or environment route:
Unverified cases:
Review result:
```

commit은 증거를 기록하고 code와 Methexis는 authority를 유지한다. 영구적인
compatibility log를 추가하거나 허용 version 목록을 문서에 복제하지 않는다.

## skill로 추출할 시점

wire 관련성과 증거에 대한 판단이 여전히 필요한 동안에는 이 내용을
가이드로 유지한다. 여러 upstream update에서 schema 생성, 집중 diff 추출,
명령 실행, report formatting 같은 안전한 기계 작업이 반복될 때만 저장소
skill로 추출한다. skill은 이 기계 작업을 자동화하고 판단을 이 가이드와
소유 KnowledgeUnit으로 돌려보내야 하며, 스스로 version을 승인해서는 안
된다.
