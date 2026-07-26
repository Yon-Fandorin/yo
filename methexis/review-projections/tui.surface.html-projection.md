---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.surface.html-projection
revision: sha256:8779702ef532b1b0761c59cb10ba935dac5fe84537f6cb9f10231c27e133cd21
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:e7579593eff55fa979c3e7ce9a49550720f184874704b331d70a348dd0308373
---
# Korean Review Projection

## Translation

초기 HTML adapter는 완성된 `Surface`를 직접 HTML/CSS fragment로 결정론적으로 투영해야 하며, 터미널 렌더링과 같은 grapheme occupancy, width profile, resolved style을 사용해야 합니다. canonical fragment는 선택적인 developer viewer·inspector chrome과 분리되어야 합니다.

초기 adapter는 ANSI나 browser terminal reflow를 흉내 내지 않습니다. 실제 debugging evidence가 operation 단위 관찰을 요구할 때 replay adapter를 추가할 수 있습니다.

직접 state projection은 agent에게 안정적이고 inspect 가능한 표현을 주고 browser 동작을 terminal truth로 오인하지 않으면서 미래 web UI를 돕습니다.
