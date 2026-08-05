---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.appearance.activity-motion-profile
revision: sha256:c094cbb2e3e56a6e80e10aef88e26fb2a55b3bacf2094b62652c798491e37624
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:5fe753b276d97796f0cfc772f9be6b62eda013a884e5b2e8c60c9c438246eb27
---
# Korean Review Projection

## Translation

Rich activity marker는 실행 중 모양이 바뀌지 않는 한 셀 글리프 `✦`를 사용하고, ASCII marker는 `*`를 사용합니다. 서로 다른 별 모양을 계속 교체하지 않으므로 터미널 폰트의 돌출 영역이 프레임마다 달라져 잘린 것처럼 보이는 현상을 피합니다.

내장 애니메이션은 정확히 16ms 간격으로 다시 그리며, 설정 가능한 profile도 16ms보다 자주 repaint하도록 만들 수 없습니다. 늦게 깨어난 경우 현재 경과 시간의 phase를 선택하고 놓친 frame을 건너뛰며 재생하지 않습니다. 적응형 cadence는 실제 runtime 근거가 생길 때 별도 scheduling 정책으로 다룹니다. 한 번의 sweep은 정확히 2초입니다. 경과 비율 `q = (elapsed mod period) / period`이고 보이는 grapheme 수가 N이면 위치는 `p = -10 + q * (N + 20)`입니다. 각 grapheme의 좌표는 0부터 N-1까지의 정수이며 p를 정수로 줄이지 않습니다. p와의 거리가 5 이내일 때 raised-cosine intensity를 사용하고 그 밖에서는 0을 사용하므로, 기존처럼 세 칸짜리 강조가 툭툭 이동하지 않고 각 글자의 밝기가 연속적으로 변합니다.

TrueColor에서는 appearance가 제공한 base와 highlight RGB 사이를 `0.9 * intensity`만큼 선형 보간하고 각 채널을 가장 가까운 정수로 반올림합니다. process host는 appearance를 공개하기 전에 color capability를 `TrueColor`, `Limited`, `Unknown` 중 하나로 명시해야 합니다. `Unknown`은 RGB를 출력하지 않는 안전한 fallback을 사용합니다. 낮은 color depth에서는 intensity가 0.2 미만이면 dim, 0.6 미만이면 기본 굵기, 그 이상이면 bold를 사용합니다. reduced motion은 marker와 문구를 정적으로 그리고 timed repaint를 요청하지 않습니다.

고정 marker는 같은 공식에 `N = 1`, `i = 0`을 넣어 같은 elapsed sample에서 색만 부드럽게 pulse합니다. 하나의 `ActivityMotionFrame` resolver가 shell의 `Working`, marker, selection panel의 typed activity title에 공통 intensity를 제공합니다. 모션은 style만 바꾸며 글자, 셀 폭, 행, fitting, 입력, 중단 동작을 바꾸지 않습니다.

Appearance candidate가 resolved color capability, marker, repaint 간격, sweep 주기, RGB endpoint, lower-depth fallback, reduced-motion 선택을 소유하고 publication 전에 검증합니다. 이 경계는 추후 terminal palette 탐지나 사용자 설정을 연결할 수 있게 하지만, 이번 revision에서 사용자 설정 파일을 공개하지는 않습니다. OSC palette 탐지는 terminal input과 timeout lifecycle을 함께 건드리므로 별도 계약으로 남깁니다.
