# yo

`yo`는 Rust 기반 코드 에이전트 인터페이스다.

현재 첫 목표는 macOS와 Linux의 최신 terminal, tmux, SSH, 원격 tmux에서
동작하는 agentic TUI 기반과 이를 사람과 에이전트가 함께 이해할 수 있는
Developer Docs를 구축하는 것이다.

기본 `yo` 호출은 대화형 TUI를 연다. 셸에서 응답 하나만 필요하면 같은
Session·Backend 경로를 `-p` 또는 `--print`로 실행할 수 있다.

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
