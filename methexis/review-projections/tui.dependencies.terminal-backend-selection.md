---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.dependencies.terminal-backend-selection
revision: sha256:632b25f826964df1a588a50e79429a721e83ef9b31002b8195083118e1800af2
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:14982a6e28b4e21346d909b99992fec6492afcd6e7899f8d8c4fa8fa888dc47b
---
# Korean Review Projection

## Translation

초기 macOS/Linux terminal adapter는 다음의 정확한 dependency surface를 사용해야 합니다. default feature를 끈 `crossterm 0.29.0`에서는 `events`, `bracketed-paste`, `event-stream`만 켜 terminal owner thread의 readiness와 key, paste, focus, mouse, resize decoding에 사용합니다. `futures-core 0.3.33`은 async runtime이나 executor를 도입하지 않고 crate-private `Stream` 경계를 직접 poll하는 용도로만 사용합니다. default feature를 끈 `rustix 1.1.4`에서는 `std`, `stdio`, `termios`만 켜 원래 TTY capture, raw-input 변경, 복구를 담당합니다. default feature를 끈 `nix 0.31.3`에서는 `signal`만 켜 private `yo-cli` process host의 typed signal mask와 기존 `sigaction` 값을 정확히 capture·restore합니다. default feature를 끈 `signal-hook 0.3.18`은 선택된 종료 signal의 기본 disposition을 문서화된 async-signal-safe 방식으로 재현하는 데만 사용합니다.

모든 dependency는 Unix target 전용이어야 합니다. Crossterm의 `serde`, `windows`, `derive-more`, `osc52`, `use-dev-tty`, Rustix의 PTY와 무관 syscall, Nix의 `signal` 이외 feature, Signal Hook의 iterator와 확장 signal 정보는 켜면 안 됩니다. Signal Hook은 Crossterm과 호환되는 dependency graph를 공유하도록 최신 `0.3` patch를 유지하며, `0.4` 이동은 별도 호환성 검토가 필요합니다. terminal owner thread가 `EventStream`을 직접 poll해 yo의 public input 값으로 변환해야 하며, 별도 terminal owner, async executor 또는 주기적 input-polling fallback을 도입하면 안 됩니다.

Surface, FrameDiff, TerminalOp, ANSI output, mode 획득 순서, cleanup 정책과 public input event는 계속 yo가 소유합니다. Crossterm, Futures Core, Rustix type은 crate-private `yo-tui::terminal::backend`에서 끝나고, Nix와 Signal Hook type은 private `yo-cli::process::termination`에서 끝나야 합니다. workspace는 unsafe Rust를 기본 거부합니다. process adapter의 격리된 disposition module만, Nix가 만든 action 또는 같은 signal에 대해 Nix `sigaction`이 돌려준 정확한 prior action을 Nix `sigaction`에 전달하는 목적에 한해 좁은 unsafe를 허용할 수 있습니다. 결정론 lifecycle test는 live terminal이 아니라 recording fake를 사용해야 합니다.

수용 조건에는 macOS/Linux compilation, 결정론 backend·partial-failure test, terminal input이 주기적 fallback 없이 무기한 대기를 깨운다는 owner-thread readiness test, unsafe 범위 검사가 포함됩니다. process coordinator state-model과 subprocess signal test는 finalization CAS의 두 결과, panic cutoff, 동시 signal 선택, handler와 `ACTIVE -> CLEANING` 경쟁, pending-bit 보존, compile-time `!Send`, 같은 thread의 mask 복구, process-lifetime handler storage, idle override, 모든 installation·shutdown failure injection 지점과 모든 `Drop` phase를 다뤄야 합니다. local modern terminal, tmux, SSH PTY, SSH 안의 tmux 환경 evidence도 필요하며 사용할 수 없는 환경은 결정론 실패와 구분해 기록해야 합니다.

Crossterm의 event decoder는 가장 좁고 성숙한 입력 경계이고, `EventStream`은 event loop나 rendering 소유권을 넘기지 않은 채 기존 terminal owner에 readiness만 제공합니다. `futures-core`도 필요한 poll trait만 제공합니다. Crossterm raw-mode helper는 mutation 성공 뒤에야 원래 termios를 저장하므로 불확실한 partial failure 이전의 보상 등록 계약을 만족하지 못하며, Rustix가 정확한 capture·restore를 위한 좁고 I/O-safe한 termios 연산을 제공합니다. Nix는 Signal Hook registry에 없는 정확한 disposition 경계를 제공하고 Signal Hook은 handler 안의 terminal write 없이 같은 signal의 기본 동작을 제공합니다. Rustix-only parser는 현대 입력 parser 유지 비용이 크고, Termwiz는 yo의 Surface·style·renderer 소유권을 중복하며, pre-1.0 Termina는 Crossterm보다 호환성 evidence가 적고 불필요한 terminal surface를 노출합니다. 환경 matrix에서 Crossterm 한계가 드러나면 Termina를 별도 검토해야 합니다.
