---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.runtime.event-source-scheduling
revision: sha256:d7f0adfae5a45b49cf70b5aa82d8b9010fe25d4c3d7fe3a8bc7aff7987b264d3
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:7bed2a4edee51bb3a2bbd1e7e9cfdca50d68a2e73e1e0a0f7b34ce6fc1fc1c6e
---
# Korean Review Projection

## Translation

live frontend는 terminal input, agent event, workspace-reference event, skill-reference event를 네 개의 일반 event source로 취급하고 live source 위에 결정적인 회전 cursor를 유지합니다. 매 선택은 cursor에서 시작해 cyclic order로 source를 살펴보고 첫 ready source를 고릅니다. 한 일반 observation을 처리하면 cursor를 그 source의 다음 source로 옮기고 다시 선택해야 합니다. 따라서 계속 ready인 어떤 일반 source도 다른 live 일반 source 각각에서 한 observation씩 처리하는 것보다 더 오래 기다리지 않으며, 이 제한은 terminal, agent, workspace, skill 모두에 대칭적으로 적용됩니다.

process termination은 이 회전에서 제외된 strict-priority control path입니다. terminal owner는 일반 source 선택 전뿐 아니라 어떤 일반 source든 poll한 뒤 그 결과를 적용하기 전에 종료를 다시 확인합니다. 어느 확인에서든 종료가 관찰되면 종료가 이기며 일반 결과를 적용하면 안 됩니다. terminal input poll 직후 확인도 여기에 포함되고 기존 cleanup과 동일 signal replay 계약을 보존합니다. suspend, 사용자 exit와 의미적 input priority는 기존 KU가 계속 소유합니다.

모든 live 일반 source와 process-termination source는 terminal owner의 waker를 등록할 수 있는 readiness를 제공해야 합니다. bounded 또는 한 항목 소비 뒤에도 buffered work가 남으면 level-ready 상태를 유지해야 하며 알림 coalescing 때문에 비어 있지 않은 queue가 stranded되면 안 됩니다. disconnect와 terminal input failure도 조용히 유실하지 않고 관찰 가능한 결과로 남깁니다.

ready source가 없으면 owner는 모든 live source에 관심을 등록한 뒤 대기합니다. frame deadline, motion deadline, active backpressure retry가 없다면 timeout 없이 무기한 기다리며 고정 input·termination·custom-provider polling fallback을 두지 않습니다. 10ms worker retry는 agent control 또는 dispatch가 실제 backpressure 상태일 때만 허용합니다. wake나 처리된 observation은 frame을 요청할 수 있지만 frame-scheduling KU를 우회하지 않습니다.

수용 조건은 first-ready cyclic selection, 동시에 계속 ready인 네 일반 source 모두에 대칭적인 one-observation 제한, 각 일반 source poll 뒤 결과 적용 전의 종료 우선순위, queue가 남을 때의 level readiness, lost notification 없는 wake 등록, 관찰 가능한 source disconnect와 terminal-input failure, 무기한 idle 대기와 active backpressure로 한정된 timed retry를 결정론적으로 검증해야 합니다. 이 의미 정책은 Crossterm과 독립적이므로 미래 GUI도 native event loop와 rendering adapter를 유지하며 재사용할 수 있습니다.
