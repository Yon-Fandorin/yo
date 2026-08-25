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
```

마지막 예처럼 stdin과 위치 prompt를 함께 주면 stdin 뒤에 prompt가 이어진다.
성공 시 stdout에는 완료된 최종 답변과 마지막 줄바꿈만 기록되고, 진행 상태·도구
활동·사용량·Session 식별자는 섞이지 않는다. 실패 진단은 stderr로 가며 종료 코드는
0이 아니다. `--model TARGET`과 새 Session 전용 `--no-tools`도 print mode에서
각각 독립적으로 사용할 수 있다.

코드를 탐색하고 검증하는 방법은
[`Developer Docs`](docs/src/README.md)에서 시작한다. 저장소 작업 방식은
[`CONTRIBUTING.md`](CONTRIBUTING.md)를 따른다.
