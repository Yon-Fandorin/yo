---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.appearance.activity-motion-profile
revision: sha256:20c8ea416043e18fe26d3aa8c0190cdb3add9ba91ea08c5427b2ee7ba3f05526
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:ed63e8a437c01a4e18e7d72f37a8577de3e80ce1c51dceb5495a3dae586078f9
---
# Korean Review Projection

## Translation

Rich 기본 로딩 마커는 rib Loader에서 검토된 Braille 순환 프레임 `⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏`을 사용하고 ASCII는 `| / - \`를 사용합니다. 기본 마커 프레임 간격은 80ms입니다. 설정된 프로필은 비어 있지 않은 프레임 문자열 목록을 제공할 수 있고, 각 프레임의 모든 완전한 grapheme cluster는 선택된 Surface 폭 프로필에서 폭 1 또는 2로 검증돼야 합니다. 프레임의 표시 셀 폭은 검증된 cluster 폭의 합으로 계산하며, appearance는 이 프레임 폭들 중 최댓값을 고정 마커 영역으로 예약합니다. 현재 프레임은 왼쪽에 배치하고 남는 오른쪽 셀은 공백으로 채우므로 폭이 다른 프레임도 `Working` 시작 위치나 fitting 결과를 움직이지 않습니다. 빈 목록·프레임, 제어 문자나 폭 0 cluster, 표현 불가능한 최대 폭, 0ms 또는 선택된 repaint 간격보다 빠른 프레임 간격은 publication 전에 거부합니다. 현재 프레임은 `floor(elapsed / frame interval) mod frame count`로 선택해 늦게 깨어나도 누락 프레임을 재생하지 않습니다.

shimmer는 기존처럼 정확히 16ms repaint와 2초 sweep을 유지합니다. activity 문구는 연속 위치와 raised-cosine intensity를 사용하고 TrueColor에서는 appearance가 소유한 RGB endpoint를 보간합니다. Limited·Unknown은 dim/default/bold fallback을 사용합니다. 마커 pulse는 같은 공식의 `N=1`, `i=0` 값을 사용하고 현재 프레임의 모든 grapheme에 동일하게 적용합니다. reduced motion은 프로필의 첫 프레임과 문구를 정적으로 표시하고 timed repaint를 요청하지 않습니다.

모션은 고정 폭 마커 영역 안에서만 프레임 내용을 바꿀 수 있습니다. 그 밖의 문구, 행과 패널 geometry, fitting, 입력과 중단 동작은 유지합니다. Appearance snapshot은 프레임 목록, 최대 예약 폭, 80ms 프레임 간격, 16ms repaint, 색상 endpoint와 fallback, motion mode를 한 revision으로 함께 고정합니다. Braille은 별 마커의 폰트 돌출 위험을 줄이지만 셀 폭 예약이 폰트의 실제 잉크 경계까지 보장하지는 않으므로 잔여 위험은 터미널 스모크로 검증합니다. 이 revision은 설정 파일을 공개하지 않지만 추후 사용자 설정이 같은 candidate 경계로 안전하게 들어올 수 있게 합니다.
