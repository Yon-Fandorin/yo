---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.overlay.selection-panel
revision: sha256:2e58f50d50fb5f38ea32ade437bf4cdb47b30f2ffd62ec728d0c957cb8fa336d
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:7dcd266e3262fac69253cdc5e5cbdbe7fddd22c82a73749af339b721864f8ddf
---
# Korean Review Projection

## Translation

첫 재사용 overlay presentation component는 Rib의 prompt completion panel을 바탕으로 한 순수 selection panel입니다. 검증된 entry와 binding hint를 받고 selected identity, viewport fitting, presentation만 소유하며 candidate 탐색이나 product effect를 실행하지 않습니다.

entry availability와 별개로 snapshot-level interaction gate가 fresh 또는 pending replacement 상태를 가집니다. pending은 마지막 fresh snapshot의 entry, selected identity, styling을 그대로 유지하지만 Tab과 Enter를 receipt나 draft 제출 없이 처리합니다. destination geometry가 그대로면 viewport도 유지하고, resize에서는 일반 fitting, selection visibility, insufficient-geometry hiding을 적용합니다. 다시 fresh가 되면 기존 선택이 여전히 enabled인 경우 보존합니다.

optional title status는 static 또는 activity presentation으로 typed되어야 하고 render code가 text를 파싱해 activity를 추측하면 안 됩니다. provider/controller는 semantic state와 안전한 text를 소유하고 panel은 검증된 presentation만 소유합니다. activity status의 sheen은 appearance로 결정되며 text나 geometry를 바꾸지 않습니다.

panel은 prompt 폭을 사용하고 muted frame, title, binding hint, selected marker, optional detail, hidden count를 표시합니다. 좁은 폭에서는 detail을 먼저 없애고 primary label을 grapheme 경계에서 자릅니다. geometry가 부족하면 원자적으로 숨깁니다.
