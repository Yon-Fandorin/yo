---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.overlay.selection-panel
revision: sha256:37c9e31dfb22598a5131695f9022be067efd1a779e6f8b416331cf8353473568
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:7a4b618de58b87c9b905617bf23fad4b7f0492c47bce749354e0edc3ae3ecf99
---
# Korean Review Projection

## Translation

첫 재사용 overlay 표시 component는 Rib의 prompt completion panel을 기준으로 한 순수 selection panel입니다. 이미 검증된 semantic entry와 실제 keymap에서 해석된 binding hint를 입력받고, 선택 identity 기준 viewport fitting과 표시를 소유하며 typed navigation outcome을 반환합니다. 후보 탐색·filter, provider query·preview 상태, 파일시스템·backend 접근, 수락된 제품 효과 실행은 소유하지 않습니다.

입력에는 안전한 비어 있지 않은 제목, semantic action caption과 짝지은 현재 물리 binding label, 한 개 이상의 순서 있는 entry가 들어갑니다. Snapshot 안 entry identity는 유일해야 합니다. 각 entry는 opaque하고 안정적인 identity, 비어 있지 않은 primary label, 선택적 detail, `enabled` 또는 reason을 가진 `disabled` availability를 가집니다. 제목·caption·label·detail·disabled reason은 publication 전에 기존의 안전한 grapheme·control-text 검증을 통과해야 합니다. Navigation은 disabled entry를 건너뛰고 accept는 disabled identity를 반환하지 않습니다. 모두 disabled인 snapshot도 selection 없이 표시하며 accept는 처리된 no-selection outcome을 반환합니다.

Panel은 prompt 폭 전체에 muted frame을 사용합니다. 위 border 왼쪽에는 제목, 오른쪽에는 현재 binding hint, 선택 행에는 appearance profile에 맞춘 marker와 accent focus, 선택적 detail에는 muted style, disabled 행과 reason에는 dim style, 보이지 않는 위·아래 항목에는 명시적 개수 표시를 사용합니다. 선택은 이동 중 항상 보여야 합니다. 넓으면 primary와 detail을 두 column으로 정렬하고, 좁으면 detail과 disabled reason을 먼저 없앤 뒤 grapheme 경계에서 primary를 자릅니다. Panel 밖으로 wrap하거나 주어진 폭을 바꾸면 안 됩니다.

사용 가능 높이는 caller가 준 destination rectangle과 component의 visible-entry cap으로 제한합니다. 위·아래 border와 entry 하나를 담지 못하면 panic이나 paint 없이 hidden outcome을 만듭니다. Resize 뒤에도 enabled selected identity가 남아 있으면 보존하고 visible window만 다시 계산합니다. 하나의 pinned appearance revision이 측정과 paint를 함께 지배합니다. Validation·준비·paint는 원자적이며 실패 시 destination Surface와 이전에 publication된 panel state를 바꾸면 안 됩니다.
