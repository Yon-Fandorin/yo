---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.chrome.input-stack
revision: sha256:9194b62f513679fff484f1f0839c09553a8678a58443966745450704d8d396c4
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:90dc80313435362d39643d056de40e592bc6d12ddef8ed68dd4106621ac6f2f6
---
# Korean Review Projection

## Translation

편집 가능한 TUI 셸의 정적 입력 크롬 순서는 일시적인 작업 영역, 프롬프트, 호스트가 실제로 아는 메트릭, 표시 모드여야 합니다. 해당 영역을 둘 수 있는 높이에서는 유휴 상태와 작업 상태가 같은 작업 영역을 예약하여 Turn 상태가 바뀌어도 프롬프트가 움직이지 않아야 합니다. 모든 영역을 담기 어려운 터미널에서는 선택적인 크롬 정보보다 프롬프트와 읽을 수 있는 최소 대화 기록을 우선합니다.

일반적으로 Turn이 활성 상태이면 작업 영역은 일반 Esc와 Ctrl+C를 모두 중단 방법으로 알려야 하며 두 키는 같은 중단 의도를 전달해야 합니다. 실제로 화면에 보이는 prompt overlay는 재배치 없이 예약된 작업 행과 인접 transcript cell을 사용할 수 있습니다. 일반 작업 행을 숨기기 전에 panel이 들어가는지 판단해야 합니다. Panel의 현재 keymap 기반 안내는 Esc 닫기와 Ctrl+C 중단을 표시해야 합니다. Esc는 overlay만 닫고, Ctrl+C는 overlay binding을 우회하여 활성 Turn을 중단해야 합니다. Overlay를 닫거나 panel 한 행도 표시할 수 없으면 snapshot이 아니라 현재 상태에서 작업 행을 복원합니다. 실제 overlay owner가 없을 때 유휴 Esc는 미처리 상태로 남고, 유휴 Ctrl+C는 입력 지우기와 두 번 눌러 종료하는 별도 정책을 유지합니다.

상태 행에는 호스트나 런타임이 실제로 아는 값만 포함해야 합니다. 알 수 없는 백엔드, 작업공간, 모델, 컨텍스트, Git 상태, 권한 값은 추측하지 않고 생략합니다. 상태는 우선순위가 있는 좌우 타입 세그먼트로 조합합니다. 셀 폭이 부족하면 값을 개행하거나 자르거나 프레임을 실패시키지 않고 세그먼트 전체를 제거합니다. 두 중단 키 이름이 들어갈 수 있다면 장식 마커보다 중단 안내를 우선합니다.

이 계약은 정적 배치와 이벤트 기반 투영만 소유합니다. 스피너 프레임, 경과 시간, 시간 기반 다시 그리기, 설정 가능한 상태줄 조합, 추가 상태 데이터 소스는 후속 계약에서 다룹니다.
