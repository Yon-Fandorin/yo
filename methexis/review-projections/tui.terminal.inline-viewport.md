---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.terminal.inline-viewport
revision: sha256:5b8ec9c7f3c0ef0eff9846d6efcc78e9cc4d9ee4390217e8544755ded8a14120
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:baf3fbf8c02ce9e66e4f720da832d3e5901c446826a9b03ff738d7213d5f0881
---
# Korean Review Projection

## Translation

Inline mode는 main screen에서 렌더링하며 terminal scrollback을 보존해야 합니다. controller는 하나의 활성 viewport와 그 바로 아래의 cursor 기준점을 소유합니다. Surface 좌표는 이 기준점에 상대적인 논리 좌표이며, 일반 렌더링이 절대 cursor 위치 질의에 의존해서는 안 됩니다.

안정된 상태에서 controller는 현재 Surface 높이에 배정된 물리 row 전체를 소유합니다. 높이가 바뀌는 동안에는 이전 영역도 계속 소유하며, 이전 높이와 현재 높이 중 큰 범위를 일관되게 맞춘 뒤 새 viewport 바로 아래로 기준점을 옮깁니다. 완료된 출력은 활성 viewport 위에 삽입해 controller가 더 이상 수정하지 않는 영속 scrollback으로 확정할 수 있습니다.

terminal 크기가 바뀌면 이전 frame을 무효화해야 합니다. 기준점과 전체 row 소유권을 계속 증명할 수 있으면 최신 completed Surface를 제자리에서 다시 그리고, 그렇지 않으면 아래의 안전 복구를 사용합니다. 중간 resize 상태는 합칠 수 있지만, 일반 resize가 영속 snapshot을 만들면 안 됩니다.

controller가 기준점이나 물리 row 소유권을 더 이상 증명할 수 없다면 증명 가능한 영역 밖을 지워서는 안 됩니다. 이전 영역은 그대로 두고 그 아래에 새 기준점을 만든 뒤 전체 redraw하며, 이 복구는 결정론적 성공이 아니라 환경 evidence로 노출해야 합니다.

이 방식은 완료된 작업을 native scrollback에 보존하면서 안정적인 입력 viewport를 제공합니다. 상대적 소유권은 terminal response protocol의 필수 의존을 피하고, 복구 시 중복 출력은 허용하더라도 사용자 소유 terminal 기록을 삭제하지 않습니다.
