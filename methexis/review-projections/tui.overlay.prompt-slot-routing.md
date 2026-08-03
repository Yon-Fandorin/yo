---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.overlay.prompt-slot-routing
revision: sha256:98716f609a3d30008068429e069ccd966edfc0f66b98b7a85ccc2c707cdabdda
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:4ba376b2e7bb5df54e09664ecfd411e91b413fe388eb1f1b5293cfd109ca0c7e
---
# Korean Review Projection

## Translation

`TuiSession`은 Chat이 보이는 동안에만 활성인 prompt-overlay slot 하나를 소유합니다. Completion과 picker provider는 query·filter·preview·effect 상태를 직접 유지하고 검증된 selection-panel snapshot만 slot에 publication합니다. Panel을 열면 이전 panel을 원자적으로 교체하고 재사용할 수 없는 overlay instance token을 반환합니다. Refresh·close·accept는 일치하는 token을 제시해야 하며, 오래되거나 다른 token의 작업은 현재 slot을 바꾸지 않고 거부합니다.

Refresh는 entry snapshot을 원자적으로 교체합니다. 기존 selected identity가 남아 있고 enabled이면 보존합니다. 아니면 안정적인 provider 순서에서 첫 enabled entry를 선택하고, 모두 disabled이면 selection을 두지 않습니다. `accept(token)`은 현재 instance와 enabled selection을 하나의 원자적 single-consumer transition에서 검증하고, 해당 instance를 되돌릴 수 없이 닫은 뒤 instance token과 opaque entry identity를 담은 acceptance receipt 하나만 반환합니다. 중복되거나 오래된 accept는 거부합니다. 제품 effect와 실패 후 retry는 provider 정책이며 닫힌 panel을 다시 accept하지 않습니다.

Process termination, job-control suspend, terminal resize, global view switch, agent-requested interaction은 prompt overlay보다 우선합니다. Chat 밖으로 전환하거나 agent-requested interaction이 publication되면 slot을 닫고, Chat으로 돌아와도 되살리지 않습니다. Chat에서는 overlay dismiss·previous·next·accept를 transcript navigation, editor, 활성 Turn interrupt보다 먼저 전달합니다. Dismiss는 일치하는 panel을 닫고 event를 소비하여 설정된 Esc 한 번이 Turn을 함께 중단하지 않게 합니다. 이 로컬 overlay action은 agent dispatch backpressure 중에도 반응해야 합니다. Ctrl+C는 provider와 overlay binding에서 예약하고 panel이 보이는 동안에도 활성 Turn 중단을 계속 전달해야 합니다. Slot이 처리하지 않은 일반 입력은 provider나 editor로 계속 전달되어 editor-attached completion이 text 변경 뒤 refresh할 수 있습니다.

Panel이 들어가는지는 transient work-status row를 숨기기 전에 판단합니다. 화면에 보이는 panel의 destination은 예약된 작업 행과 인접 transcript cell이며, 재배치 없이 움직이지 않는 prompt 바로 위에 bottom-anchor합니다. Panel을 닫거나 한 행도 표시할 수 없어 숨기면 snapshot이 아니라 현재 상태에서 work-status row를 다시 렌더합니다. Prompt·footer·현재 frame 밖을 덮지 않으며 open·replace·resize·close로 prompt나 footer geometry를 움직이면 안 됩니다. Panel을 표시할 수 없으면 slot state는 유지할 수 있지만 hidden 상태에서 입력 우선권을 주장하지 않습니다. 모든 입력을 소유하는 modal, nested overlay, overlay stack은 이번 범위 밖입니다.
