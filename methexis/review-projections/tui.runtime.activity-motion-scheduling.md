---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.runtime.activity-motion-scheduling
revision: sha256:669870ee6ec281b4155ff43d5cca2950e2d62adb0ddefba9b82d53f20b642097
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:80ad1ec4a0b78dcde7054a1e10090ff01db0a1b22c129f0c4a1a111335777497
---
# Korean Review Projection

## Translation

각 터미널 소유 세대마다 live runner가 단 하나의 monotonic animation epoch를 소유합니다. 상태, 컴포넌트, 완성된 Surface, terminal presenter, HTML projection은 직접 시계를 읽거나 sleep하거나 animation thread를 만들지 않습니다. Frame preparation만 해당 epoch에서 계산한 명시적인 elapsed sample을 받습니다.

완성된 frame은 동적 activity marker를 실제로 그렸을 때만 typed motion demand를 반환합니다. Turn이 active라는 이유만으로 timer를 켜지 않으며, 다른 view이거나 높이·폭이 부족해 marker가 보이지 않거나 idle·zero-size라면 timed redraw를 하지 않습니다.

frame period를 P라고 할 때 tick은 floor((now-epoch)/P), 다음 deadline은 epoch+(tick+1)P입니다. 늦게 깨어나면 놓친 tick을 건너뛰고 현재 tick 하나만 그리며 catch-up redraw를 연속 실행하지 않습니다. event redraw와 animation deadline이 겹치면 현재 sample로 frame을 한 번만 준비합니다. backpressure를 포함한 모든 input wait는 input, termination, retry 응답성을 약화하지 않으면서 활성 deadline을 함께 고려합니다.

zero-size 동안 epoch는 유지하지만 render는 억제합니다. 다시 보이는 첫 frame은 그 시점의 tick을 사용합니다. suspend/resume은 semantic Turn 상태는 유지하지만 새 terminal generation에 새 epoch를 만듭니다. Inline과 Fullscreen은 event와 animation 모두 같은 prepared Surface·presenter 경로를 사용합니다. terminal-independent frame preparation만 명시적인 elapsed sample을 받을 수 있습니다. Presenter와 HTML은 완성된 Surface만 소비하며 스스로 motion을 진행시키지 않고, archival output은 activity chrome을 포함하지 않습니다.
