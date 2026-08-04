---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.runtime.activity-motion-scheduling
revision: sha256:4b7ecbd05bd0936adc1f49073c392f5428e8b566101d27b06a9e05944a5a50c5
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:92c48218fa91149e5fffa63111c6615b6041a732d5fb628559843540d8dc5032
---
# Korean Review Projection

## Translation

live terminal ownership generation마다 runner가 monotonic animation epoch 하나를 소유합니다. state, component, 완성된 Surface, presenter, HTML projection은 clock을 읽거나 sleep하거나 animation thread를 만들지 않고, frame preparation은 epoch에서 얻은 명시적 elapsed sample을 받습니다.

완성 frame은 실제로 보이고 다음 논리 frame에서 완성 cell 하나 이상을 바꿀 수 있는 dynamic indicator를 그렸을 때만 typed motion demand를 보고합니다. Turn이 active이거나 provider가 pending이라는 사실만으로 redraw를 예약하면 안 됩니다. 다른 view, overlay 상태, 부족한 높이 또는 폭 때문에 indicator가 숨겨지면 timed redraw도 없어야 합니다. 여러 indicator가 보이면 가장 짧은 양수 period를 보고하며, idle과 0 크기 surface는 redraw를 해제합니다.

마감 시각은 epoch와 period로 계산하고 늦게 깨어나면 놓친 frame을 재생하지 않습니다. event redraw와 motion redraw는 하나로 합치며 모든 input wait 경로가 deadline을 존중합니다. suspend/resume은 새 terminal generation과 epoch를 만들고 retained Turn 상태는 보존합니다. terminal-independent frame은 elapsed sample을 주입받을 수 있지만 presenter와 HTML은 완성 Surface만 소비합니다.
