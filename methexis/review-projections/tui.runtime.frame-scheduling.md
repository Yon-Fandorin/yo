---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.runtime.frame-scheduling
revision: sha256:f356fb179b08f615ceaf5d018a89ba8a4fe204b04933766b5851f6fa097bcf56
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:76f67ae23c21aa2049f5b98bd575d062db531dbceb4a497240006202836bd8a1
---
# Korean Review Projection

## Translation

live terminal ownership generation마다 runner가 frame scheduler 하나를 소유합니다. terminal input, agent, workspace, skill, motion에 따른 의미 상태 변화는 직접 표시하지 않고 공통 scheduler에 frame을 요청하며, 일반 요청은 하나의 frame 경계에서 합쳐집니다.

일반 frame의 기본 coalescing cadence는 120fps이고 host가 60fps를 선택할 수 있습니다. 이번 revision은 60과 120만 지원합니다. 완성된 일반 frame 이후 다음 coalesced frame은 선택한 fps로 계산한 최소 간격이 지난 뒤 표시됩니다. terminal generation의 첫 visible frame과 resize가 요청한 frame은 올바른 초기 화면과 geometry 복구를 위한 명시적 예외이므로 이 경계를 기다리지 않고 즉시 표시합니다. 따라서 선택한 cadence는 일반 coalesced frame을 제한하며 이 두 즉시 경우까지 전역적으로 제한하지 않습니다. terminal 크기가 0이면 표시 작업을 억제합니다.

CLI의 `tui.max_fps`는 숫자 60 또는 120만 받고 기본값은 120이며 나머지는 명시적으로 거부합니다. 역사적인 이름은 `max_fps`지만 일반 coalescing cadence를 선택하며 first·resize 예외는 그대로 유지됩니다. live startup에서 repeatable terminal generation loop를 열기 전에 한 번 읽고 suspend/resume 뒤에도 같은 값을 유지합니다. 실행 중 reload는 이번 범위 밖입니다.

motion deadline이 도착하면 deadline을 지우기 전에 보존되는 coalesced frame 요청으로 바꿉니다. 그 뒤에는 이미 소비한 motion deadline이 아니라 frame boundary까지 기다려야 하므로 60fps 설정에서 더 빠른 motion 주기 때문에 zero-timeout loop가 생기지 않습니다.

수용 조건은 기본·선택 일반 interval, 일반 요청 coalescing, first·resize frame의 명시적인 즉시 우회, due-motion 보존, zero-size 억제, 설정 검증과 terminal generation 사이 startup 선택 보존을 검증해야 합니다. 이 정책은 Crossterm에 묶이지 않으며 미래 GUI는 native event loop나 display synchronization에 맞춰 표시하면서 coalescing 의미를 재사용할 수 있습니다.
