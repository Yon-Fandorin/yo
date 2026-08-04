---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.appearance.glyph-profiles
revision: sha256:fcdba9627e4dda4e968aa24837993d2b2247ba9e30a511781ffb6f3a9db334d9
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:ee65af9cbc85e36de3a7144f3973d6ca055d3a33c3ba76660e66705e53afe5bf
---
# Korean Review Projection

## Translation

초기 모양새 어휘는 대화 기록에서 사용자 표식을 Rich의 `❯`, ASCII의 `>`로, 어시스턴트 표식을 Rich의 `•`, ASCII의 `*`로 정확히 제공해야 합니다. Rich가 기본이며 ASCII는 명시적인 세션 모양새 후보를 통해서만 선택합니다. 초기 구현은 `TERM`, 색상 기능, `NO_COLOR`로 글리프 프로필을 추론하지 않습니다.

Rich 어시스턴트 표식을 `•`로 바꾸는 것은 아직 릴리스 전인 화면과 일반 출력 바이트에 대한 의도적인 표현 변경입니다. 메시지 역할은 렌더링의 시맨틱 입력으로 유지하며, 이전에 렌더링한 표식을 영속 신원으로 보존하지 않고 선택된 모양새 스냅샷이 현재 표식을 제공합니다.

각 후보 표식은 비어 있지 않은 확장 그래핌 클러스터 하나여야 하고, 게시 전에 제어 문자, ANSI 내용, 폭이 0인 클러스터를 거부해야 합니다. 폭 측정은 별도 표가 아니라 기존 `yo-unicode-17.0-narrow/v1` Surface 폭 소유자를 사용하며, 승인된 표식은 설정된 본문 들여쓰기 안에 들어가야 합니다.

Rich와 ASCII 표식의 셀 폭은 같을 필요가 없습니다. 레이아웃은 공통 들여쓰기 안에서 보정하여 모든 프로필에서 사용자와 어시스턴트 본문이 같은 설정 열에서 시작하게 해야 합니다. 화면과 일반 세션 출력은 같은 확정 스냅샷에서 표식을 얻어야 합니다.
