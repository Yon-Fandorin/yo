---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.terminal.inline-viewport
revision: sha256:dad59a7d08cb9afc9466bdc9e675d2d497d51f623b24a7b726c0cb01fd61cb2c
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:e21e2710d15ce9710602b5f730afb33e1fe57e868f0a8d26e22273af9f27d095
---
# Korean Review Projection

## Translation

# Inline 소유 viewport

## 명세

Inline 모드는 main screen에 렌더링하고 terminal scrollback을 보존해야 합니다. 하나의 활성 viewport와 그 바로 다음에 있는 논리적 bottom anchor를 소유해야 합니다. Surface 좌표는 viewport 기준 논리 좌표입니다. frame 사이에는 terminal 자체 입력기의 후보 창이 보이는 caret을 따를 수 있도록 물리 terminal cursor를 viewport 안의 prompt caret에 둘 수 있습니다. controller는 그 caret을 viewport 상대 좌표로 기억하고 상대 이동 제어로 자신이 소유한 좌표계로 돌아와야 합니다. 일반 렌더링은 절대 cursor 위치 조회를 요구해서는 안 됩니다.

정상 상태에서 controller는 현재 Surface 높이에 할당된 물리 행 전체를 소유합니다. 높이가 바뀌는 동안에는 이전과 현재 높이 중 큰 범위를 정리할 때까지 이전 footprint를 계속 소유하고, 새 viewport 바로 아래로 논리적 anchor를 옮깁니다. 완료된 출력은 활성 viewport 위에 삽입할 수 있으며, 이후 controller의 변경 영역 밖에 있는 영속 scrollback이 됩니다.

controller는 redraw 중 물리 cursor를 숨기고, 완전한 frame이 flush된 뒤에만 현재 prompt caret에서 다시 보여야 합니다. 그리기 전에 cursor visibility 복구를 terminal lifecycle 소유권에 등록해야 하므로 정상 종료, 렌더링 실패, panic cleanup 모두 cursor를 보이게 두는 복구를 시도합니다.

terminal geometry가 바뀌면 이전 frame은 무효가 되어야 합니다. 논리적 anchor와 전체 행 소유권을 증명할 수 있으면 최신 완성 Surface를 제자리에서 다시 그리고, 그렇지 않으면 아래 복구 절차를 사용합니다. 대체 가능한 중간 resize 상태는 합칠 수 있습니다. 일반 resize는 영속 snapshot을 만들면 안 됩니다.

controller가 논리적 anchor, 기억한 caret 또는 물리 행 소유권을 더 이상 증명할 수 없으면 증명 가능한 영역 밖을 지우면 안 됩니다. 그 영역을 포기하고 그 아래에 다시 anchor를 잡아 전체를 다시 그리며, 이 복구를 결정론적 성공이 아니라 환경적 evidence로 노출해야 합니다.

## 근거

경계가 있는 변경 가능 viewport는 agent 상호작용에 안정적인 composer를 제공하면서 완료된 작업을 native scrollback으로 남깁니다. 상대적 소유권은 필수 terminal 응답 protocol을 피합니다. 명시적인 복구 경로는 사용자 소유 terminal history를 지우는 것보다 진단 출력이 중복되는 편을 택합니다. 실제 prompt caret은 terminal-native IME 위치를 맞추고, lifecycle에 등록된 visibility 복구는 부분 출력 실패가 cursor를 숨긴 채 남기는 것을 방지합니다.
