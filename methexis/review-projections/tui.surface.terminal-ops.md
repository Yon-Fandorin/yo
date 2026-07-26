---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.surface.terminal-ops
revision: sha256:ad84fe74ecf5998e0f5f20c92ac793d02550bc114b0836d580d916b47d63c1b1
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:94baa4e71cbff78eb16c6eb982bc148fa3b75be323cf3f38dfdd3ba187acd3fa
---
# Korean Review Projection

## Translation

터미널 렌더링은 `FrameDiff -> TerminalOp -> ANSI encoder` 흐름을 따라야 합니다. `TerminalOp`는 cursor 이동, resolved style 선택, grapheme write 같은 효과를 byte encoding 전에 typed value로 표현해야 합니다.

Inline과 Fullscreen은 `Surface`, diff, terminal operation semantic을 공유해야 합니다. 바깥 mode controller만 terminal 진입·복구·cursor 정책을 소유해야 합니다.

typed intermediate boundary는 실제 터미널 없이 순서를 테스트하게 하고 mode lifecycle의 side effect가 재사용 가능한 UI state로 새는 일을 막습니다.
