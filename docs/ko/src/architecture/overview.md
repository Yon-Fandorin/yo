# 아키텍처

현재 `yo`는 프런트엔드에 독립적인 코어, TUI 라이브러리, 프로세스
진입점으로 구성된다.

| 영역 | 소유하는 책임 | 탐색 시작점 |
|---|---|---|
| `yo-cli` | 프로세스 시작, Unix 종료 조율, Inline 또는 Fullscreen 표시 방식 선택 | [`crates/yo-cli/src/main.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/main.rs) |
| `yo-core` | 에이전트 세션 의미, 명령과 이벤트, 백엔드 포트, Codex app-server 어댑터 | [`crates/yo-core/src/lib.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/lib.rs) |
| `yo-tui` | 입력 편집, 대화 기록 레이아웃, 터미널 모드와 렌더링, 공유 HTML Projection | [`crates/yo-tui/src/lib.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/src/lib.rs) |

의존성은 다음 방향으로만 흐른다.

```text
yo-cli
├── yo-core
└── yo-tui
    └── yo-core
```

`yo-core`는 프런트엔드에 의존하지 않는다. 따라서 추후 GUI도 터미널
정책을 떠안지 않고 같은 에이전트 세션 경계를 사용할 수 있다.

## 계약 소유자

- [프런트엔드 독립 코어 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.core.frontend-independent-boundary.md)
- [TUI 전용 크레이트 경계](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.crate.ui-only-boundary.md)
- [모듈 경계 정책](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/tui-architecture/tui.architecture.module-boundaries.md)

어느 모듈이 변경을 소유하는지 고르려면 [코드 지도](./code-map.md)로
이동한다. 한 번의 사용자 요청이 실행되는 전체 경로를 보려면
[실행 흐름](./runtime-flow.md)으로 이동한다.
