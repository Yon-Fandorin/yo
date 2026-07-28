---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.dependencies.terminal-backend-selection
revision: sha256:263b9b9a27a07bbcae8c1bb5bb6144fba5ef69e7857ca0698152bbf458692312
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:8ca7f1948068f5c12b0b65c3affcf58f3476266260ba7f2cf6c906cb6bf489f3
---
# Korean Review Projection

## Translation

초기 macOS/Linux adapter는 default feature를 끈 `crossterm 0.29.0`의 `events`, `bracketed-paste`로 동기 input을 decode하고, default feature를 끈 `rustix 1.1.4`의 `std`, `stdio`, `termios`로 원래 TTY capture, raw 변경, 복구를 수행합니다. private `yo-cli` process host는 default feature를 끈 `nix 0.31.3`의 `signal`로 typed mask와 기존 sigaction을 정확히 capture/restore하고, feature 없는 `signal-hook 0.3.18`은 선택한 종료 signal의 async-signal-safe 기본 동작 재현에만 사용합니다. 이 선택은 process coordinator와 terminal lifecycle 계약을 함께 충족해야 합니다.

모든 dependency는 Unix target 전용입니다. Crossterm의 event-stream, serde, windows, derive-more, osc52, use-dev-tty, Rustix의 PTY·무관 syscall, Nix의 signal 이외 feature, Signal Hook iterator·확장 signal 정보는 켜지 않습니다. Signal Hook 0.4 이동은 별도 검토합니다.

Surface, FrameDiff, TerminalOp, ANSI output, mode 획득 순서, cleanup, public input event는 yo가 소유합니다. Crossterm/Rustix type은 crate-private `yo-tui::terminal::backend`에서, Nix/Signal Hook type은 private `yo-cli::process::termination`에서 끝납니다. workspace는 unsafe Rust를 기본 거부하고, process adapter의 격리된 disposition 모듈만 같은 signal의 Nix action을 전달하는 좁은 unsafe를 허용합니다. 결정론 lifecycle test는 live terminal 대신 recording fake를 사용합니다.

수용 조건은 macOS/Linux compile, backend·partial failure test, coordinator state-model과 subprocess test, unsafe 범위 검사, local terminal/tmux/SSH PTY/SSH 내부 tmux evidence입니다. coordinator test는 finalization CAS 두 결과, panic cutoff, 동시 signal 선택, handler와 `ACTIVE -> CLEANING` 경쟁, pending bit 보존, compile-time `!Send`, 같은 thread mask 복구, process-lifetime handler storage, idle override, 모든 installation·shutdown failure 지점과 Drop phase를 포함합니다. 사용할 수 없는 환경은 결정론 실패와 구분합니다.

Rustix는 Crossterm raw helper가 보장하지 못하는 partial-failure 이전 보상 등록을 위한 좁은 termios 연산을 제공합니다. Nix는 정확한 disposition 경계를, Signal Hook은 handler 안에서 terminal write 없이 같은 signal 기본 동작을 제공합니다. Rustix-only parser, Termwiz, Termina 대안은 기존 결론을 유지하며 환경 검증에서 Crossterm 한계가 드러나면 Termina를 다시 검토합니다.
