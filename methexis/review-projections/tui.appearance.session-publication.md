---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.appearance.session-publication
revision: sha256:5419dabeac274a7afded46caa29fc07474a1f6406e61f4f1d08787483c199551
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:239f2895e8245a13bce7a6bed4a74e4d0f431bb47724f0571395b6507dbe511d
---
# Korean Review Projection

## Translation

각 `TuiSession`은 완전히 resolve된 style, glyph, layout 설정을 담은 불변 committed appearance snapshot 하나와 단조 증가 revision을 소유해야 합니다. 물리 `Surface` cell은 지금처럼 최종 resolved `Style`만 저장합니다.

TUI owner thread만 이 값을 쓸 수 있습니다. logical frame 준비 밖에서 candidate 전체를 검증하고 resolve해야 하며, 잘못된 candidate는 명시적인 거부를 반환하고 기존 snapshot과 revision을 모두 보존해야 합니다. 유효한 candidate는 snapshot 전체를 원자적으로 교체하고 revision을 올린 뒤 다음 logical frame부터 보여야 합니다. 게시된 일부 필드만 따로 바꾸면 안 됩니다.

appearance 선택을 process-global mutable state, thread-local current scope, 숨은 string-key lookup에 두면 안 됩니다. committed snapshot과 revision은 terminal suspend/resume 뒤에도 유지합니다. generation마다 presenter history는 새로 만들 수 있지만, 새로운 generation 전용 capability 입력이 없다면 appearance를 다시 추측해 resolve하지 않습니다.

초기 runtime replacement seam은 crate-private이어도 됩니다. public appearance API는 실제 host consumer와 문서 계약을 별도로 검토한 뒤 추가합니다.
