---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.dependencies.terminal-backend-selection
revision: sha256:0649b47073f85abba9378aba0f7db21280626a7ba18fe4bd371d4da39b3f383e
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:8ba82841b7b757f980e5a8aef3aa330b447839d39780f40fb4c1ae2217ceabe4
---
# Korean Review Projection

## Translation

초기 macOS/Linux terminal adapter는 다음 의존성 표면만 사용해야 합니다.

- `crossterm 0.29.0`: default feature를 끄고 `events`, `bracketed-paste`만 켭니다. 동기식 input polling과 key, paste, focus, mouse, resize event 해석만 맡깁니다.
- `rustix 1.1.4`: default feature를 끄고 `std`, `stdio`, `termios`만 켭니다. Crossterm이 Unix에서 불가피하게 켜는 Rustix 표면과 일치시키며, terminal을 바꾸기 전에 원래 TTY 속성을 정확히 저장하고 raw input 상태를 적용하거나 원래 상태로 복구하는 데 사용합니다.
- `signal-hook 0.3.18`: default feature를 끄고 `iterator`만 켭니다. 등록한 Unix 종료 signal을 typed control path로 전달하고 terminal 복구 뒤 같은 signal의 기본 동작을 실행하는 데 사용합니다.

세 의존성은 Unix target에만 둡니다. Crossterm의 async stream, serde, Windows, helper derive, clipboard, `/dev/tty` 전용 polling feature와 Rustix의 PTY 및 무관한 syscall feature, Signal Hook의 확장 signal 정보 feature는 초기 범위에서 켜지 않습니다. Signal Hook은 Crossterm과 하나의 호환 dependency graph를 사용하기 위해 최신 `0.3` patch를 선택하며, `0.4` 전환은 별도 호환성 검토가 필요합니다.

`Surface`, `FrameDiff`, `TerminalOp`, ANSI 출력, mode 진입 순서, cleanup 정책, 공개 input event의 소유권은 yo에 남습니다. 외부 crate 타입은 crate-private `terminal::backend` adapter 밖으로 나오면 안 됩니다. 결정론적 lifecycle 테스트는 실제 terminal 대신 같은 내부 backend trait을 구현한 recording fake를 사용합니다.

수용 조건은 macOS/Linux compile, backend와 partial-failure 결정론 테스트, 최신 local terminal, tmux, SSH PTY, SSH 안의 tmux 환경 evidence입니다. 사용할 수 없는 환경 항목은 결정론 실패로 섞지 않고 별도로 표시합니다.

Crossterm은 성숙한 event decoder를 제공하지만 raw-mode helper는 변경이 성공했다고 보고된 뒤에야 원래 termios를 저장하므로, 결과가 불확실한 partial failure 전에 복구 의무를 등록해야 하는 yo 계약을 단독으로 만족시키지 못합니다. Rustix는 custom input parser 없이도 정확한 TTY 저장과 복구에 필요한 좁고 I/O-safe한 termios 연산을 제공합니다. Signal Hook은 비동기 handler에서 terminal을 직접 쓰지 않고 signal 전달과 같은 signal의 기본 종료 동작을 제공합니다.

Rustix만 사용하면 modern keyboard, paste, mouse, focus, resize parser를 yo가 직접 유지해야 합니다. Termwiz는 yo가 이미 소유한 Surface, cell, style, renderer와 겹치며 API 변동 범위도 더 큽니다. Termina는 terminal protocol을 드러내는 lower-level parser를 제공하므로 가장 가까운 현대적 교체 후보지만, 아직 초기 pre-1.0 adapter surface라 Crossterm보다 누적 호환성 evidence가 적고 yo가 필요로 하지 않는 escape, style, terminal ownership도 함께 노출합니다. 추천 조합은 외부 라이브러리를 typed yo 값 뒤에서 교체할 수 있게 유지하면서 큰 정책 표면을 가져오지 않습니다. 초기 environment matrix에서 Crossterm 제약이 확인되면 Termina를 별도 호환성 검토로 재평가해야 합니다.
