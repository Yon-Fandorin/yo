---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.terminal.inline-viewport
revision: sha256:4993cb2e43b2df320b358339a058581d0963f1cd310c28b2ef106d04f8b42360
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:3147a7ae64b9cf94c2c2b12ea9ff1fac53941c6b94eea0795a6f98057fa8843e
---
# Korean Review Projection

## Translation

# 컴팩트 인라인 라이브 영역

## 명세

Inline 모드는 main screen에 렌더링하고 터미널 기본 scrollback을 보존해야 합니다. Chat projection은 순서가 보존된 전체 의미 이력을 유지하고, 단조롭게 전진하는 scrollback 발행 cursor를 별도로 소유해야 합니다. 발행 후보는 아직 발행되지 않은 `Final` Chat transcript 항목의 최대 연속 prefix여야 합니다. 항목의 일부만 발행하거나, 앞선 `Streaming` 항목을 건너뛰거나, 보이는 행 위치를 정체성으로 사용해서는 안 됩니다. 후보 경계는 안정적인 transcript 항목 ID와 최종 revision으로 마지막 항목을 식별해야 합니다.

편집 가능한 `FollowTail` Chat에서 frame 준비는 하나의 고정된 appearance와 관찰한 터미널 너비를 사용해 후보의 영속 행과, 아직 발행되지 않은 suffix 및 현재 prompt, chrome, overlay를 담은 컴팩트 live Surface를 함께 구성해야 합니다. composer는 발행 경계를 가로지르는 separator와 기타 formatting을 소유해야 합니다. live Surface 높이는 측정한 자연 높이를 사용 가능한 터미널 높이로 제한한 값이어야 하며, 공간이 있다는 이유만으로 모든 행을 차지해서는 안 됩니다.

발행은 prepare, present, observe, acknowledge transaction입니다. 준비 단계에서 발행 cursor를 전진시켜서는 안 됩니다. 준비된 plan은 예상 이전 cursor, 후보 경계, appearance, 관찰한 전체 터미널 크기와 단조로운 geometry epoch를 결속해야 합니다. 터미널을 소유한 controller는 크기가 이전 값으로 되돌아오는 알림을 포함해 자신이 관찰한 모든 resize 알림마다 epoch를 전진시켜야 합니다. Inline presenter는 영속 행을 활성 viewport 바로 위에 삽입하고, 소유한 live footprint를 조정하고, prompt caret을 배치하고, 전체 plan을 flush해야 acknowledgement를 고려할 수 있습니다. flush 후 controller는 이미 전달된 resize 알림을 block 없이 관찰하고 터미널 크기를 다시 표본화해야 합니다.

이 flush 후 관찰은 영속 발행과 live viewport 소유권을 별도로 판정해야 합니다. 모든 후보 operation이 parser state와 operation 경계가 확정된 상태로 터미널 stream에 들어갔고 effect ledger가 그 완전한 효과를 증명할 때만 영속 발행이 완료됩니다. 관찰한 epoch와 표본 크기가 준비된 plan과 여전히 일치할 때만 live viewport 소유권이 현재 상태입니다. 두 조건이 모두 참이면 controller는 후보를 acknowledge하고 live frame을 commit해야 합니다. 영속 발행은 완료됐지만 geometry가 오래된 경우에는 영속 효과를 지우거나 replay하지 않은 채 후보 경계를 acknowledge하고, 이미 기본 이력으로 이동한 행을 보존하고, 준비된 live frame과 그 물리적 소유권을 거부한 뒤 새 geometry에서 의미적으로 아직 발행되지 않은 live suffix와 interactive chrome만 다시 준비해야 합니다. 이 분리 acknowledgement는 내용 전달 완료를 주장할 뿐 이전 layout의 소유권을 주장하지 않습니다. 영속 발행이 불완전하거나 증명할 수 없으면 의미 cursor를 그대로 두고 아래의 제한된 effect-ledger 보정에 들어가야 합니다. geometry 불일치만으로 완료가 증명된 영속 prefix를 replay해서는 안 됩니다. 플랫폼이 이 관찰 경계 뒤에야 드러내는 resize는 다음 관찰에 속하고, acknowledge된 영속 prefix를 소급해 바꿀 수 없으며, live frame과 더 이상 증명할 수 없는 물리적 소유권을 무효화해야 합니다.

state는 위 규칙에 따른 완전한 영속 발행 receipt가 있을 때만 발행 cursor를 전진시켜야 합니다. 전체 plan과 일치하는 receipt만 준비된 live frame도 commit합니다. 준비 실패, 발행 전 이미 오래된 것으로 확인된 plan, 완전한 영속 발행 receipt가 없는 plan, 또는 write나 flush 실패는 의미 cursor를 그대로 두고 불확실한 물리적 소유권을 신뢰할 수 없게 만들어야 합니다. 성공이 증명된 transaction에서는 각 발행 항목이 기본 scrollback에 정확히 한 번 나타납니다. 터미널 일부 쓰기로 성공 여부를 알 수 없으면 의미 cursor는 그대로 유지되어야 합니다.

발행 byte는 presenter가 소유한 unbuffered 터미널 transport를 통과해야 합니다. 보고된 각 write count는 downstream 터미널 stream에 들어간 정확한 byte prefix를 식별해야 하며, `flush`가 숨겨진 buffered byte를 새로 전달해서는 안 됩니다. 그 transport 위 계층의 writer acceptance는 전달 증거가 아닙니다. 활성 transport가 이러한 속성을 제공하지 못하면 write 또는 flush 실패를 전달된 prefix를 알 수 없는 상태로 취급해야 하며 erase-or-resume 보정에 들어가서는 안 됩니다.

presenter는 발행을 순서가 있고 자체 경계를 식별할 수 있는 terminal operation으로 encode해야 하며, transaction 동안 예상 operation byte, 예상 cell 행, 완전한 operation 단위의 터미널 stream 진행도, parser 경계, geometry epoch, anchor, cursor와 물리적 효과를 유지해야 합니다. effect ledger는 주소 지정 가능한 main-screen write 및 erase를 scrolling, insertion, deletion 또는 출력을 기본 이력으로 이동시킬 수 있는 모든 operation과 구분해야 합니다. text가 일치한다는 사실만으로 소유권을 확립해서는 안 되며, 이 설계가 이식성 없는 터미널 screen 또는 scrollback readback을 요구해서도 안 됩니다.

발행 출력 오류는 원래 오류를 primary로 보존하고 의미 cursor를 그대로 두며, rendering failure가 되기 전에 최대 한 번의 제한된 보정을 시도해야 합니다. 터미널 stream prefix가 완전한 operation에서 끝나 parser state와 operation 경계가 확정되어 있고 ledger가 geometry, anchor, cursor와 소유권을 여전히 증명하면, controller는 즉시 중복하는 대신 적용 가능한 정확한 보정을 선택해야 합니다. 완료된 prefix의 모든 효과가 현재 주소 지정 가능하고 소유권이 증명된 main-screen 행 안에서 되돌릴 수 있으면, 그 효과를 지우고 깨끗한 footprint에서 준비된 전체 발행을 다시 시작해야 합니다. 효과를 되돌릴 수 없지만 정확히 완료된 prefix, 그 결과 cursor와 남은 operation suffix를 알고 있다면, 그 prefix를 지우거나 반복하지 않고 보존한 채 남은 suffix만 이어서 처리해야 합니다. 두 경우 모두 복구된 전체 plan 완료, flush 및 일치하는 flush 후 geometry 관찰을 마쳐야만 후보를 acknowledge하고 session을 계속할 수 있습니다. 복구는 환경 증거로 노출되어야 합니다.

정확한 보정이 불가능하지만 parser state와 operation 경계가 확정되어 있고 controller가 영향받은 영역을 안전하게 포기한 뒤 새로운 소유 viewport를 설정할 수 있으면, 그 위치에서 의미적으로 아직 발행되지 않은 전체 plan을 다시 실행할 수 있습니다. 이 최후 수단 경로에서만 이전 prefix가 중복될 수 있습니다. replay가 성공하면 완전한 plan을 acknowledge하고 session을 계속할 수 있습니다. operation 일부만 터미널 stream에 들어갔거나, 숨겨진 하위 계층 전달 가능성이 있거나, parser state와 operation 경계가 확정되지 않았거나, 안전한 새 viewport를 만들 수 없거나, 한 번의 복구 write 또는 flush가 실패하면 controller는 복구를 중단해야 합니다. 원래 발행 오류는 primary rendering failure로 남고, 보정 실패는 추가 진단 증거로 첨부되며, 기존 터미널 lifecycle restoration이 실행됩니다. 어떤 복구 경로도 소유하지 않은 행을 지워서는 안 됩니다. 이 보장은 의미적 발행 무결성에 관한 것이며, 복구 불가능한 프로세스 경계를 넘는 가시적 전달을 보장하는 것은 아닙니다.

발행된 scrollback 행은 변경할 수 없는 터미널 이력이며 일반 resize 중 reflow, clear 또는 replay해서는 안 됩니다. 활성 viewport는 새 너비에서 layout을 다시 계산하고, 논리적 bottom anchor를 이동하기 전에 이전과 현재의 소유권이 증명된 높이 중 큰 footprint를 조정해야 합니다. 대체 가능한 중간 resize 상태는 병합할 수 있습니다. anchor, caret 또는 전체 행 소유권을 더 이상 증명할 수 없으면 controller는 그 영역을 포기하고 그 아래에 새 anchor를 만든 뒤, 영속 출력이나 소유하지 않은 출력을 지우지 않고 아직 발행되지 않은 live 영역을 완전히 다시 그려야 합니다.

Chat이 tail에서 분리되어 있거나 읽기 전용 Transcript 또는 Request view가 활성화되어 있으면, Inline은 탐색 중 전체 이력이 계속 보이도록 발행을 멈춰야 합니다. 이 review viewport는 사용 가능한 터미널 높이를 사용할 수 있습니다. 편집 가능한 `FollowTail` Chat으로 돌아오면 새로 자격을 얻은 prefix를 발행하고 다시 컴팩트 live 높이로 줄일 수 있습니다. Fullscreen은 항상 전체 의미 이력을 렌더링해야 하고 Inline 발행 cursor를 무시하며 절대 전진시켜서는 안 되고, 기존 screen 및 종료 동작을 유지해야 합니다.

일반 Inline 종료와 typed termination은 먼저 터미널 lifecycle을 복원한 다음, 아직 발행되지 않고 유지된 Chat suffix만 선택적인 caller 소유 session output으로 노출해야 합니다. 그 suffix는 발행 경계의 separator와 committed appearance snapshot을 포함해 동일한 transcript formatting 계약을 사용해야 합니다. suspend 전환은 session output을 내보내지 않고 의미 발행 cursor를 보존해야 하며, resume 뒤 새로 획득한 viewport에 아직 발행되지 않은 live 영역을 다시 그려야 합니다. 그 밖에는 출력할 수 있는 종료에서 restoration이 마지막 물리 발행의 완료 여부를 증명하지 못하더라도, 종료 suffix는 의미적으로 아직 발행되지 않은 모든 항목을 포함해야 하며 보이는 출력이 반복될 수 있습니다.

위의 제한된 보정으로 완전히 복구된 출력 오류는 rendering-failure 결과가 아닙니다. 복구되지 않은 rendering failure, panic과 cleanup failure는 기존 lifecycle 실패 및 진단 처리를 유지해야 하며, 선택적인 session output을 내보내기 위해 성공 종료로 다시 분류해서는 안 됩니다. 이러한 치명적 경로는 아직 발행되지 않은 suffix를 출력할 의무가 없습니다. 다만 acknowledge되지 않은 발행이 실패 전에 의미 cursor를 전진시키지 않았음을 보장해야 합니다.

controller는 viewport를 변경하는 동안 물리 cursor를 숨기고, 완전한 plan이 flush된 뒤에만 현재 prompt caret에서 이를 보여야 합니다. 그리기를 시작하기 전에 cursor visibility restoration을 터미널 lifecycle 소유권에 등록해야 합니다. 일반 렌더링은 기억한 caret과 상대 제어를 사용해야 하며 절대 cursor 위치 조회를 요구해서는 안 됩니다.

## 근거

유지된 의미 이력과 단조로운 물리 발행 cursor를 분리하면 Fullscreen 이력이나 진단 탐색을 약화하지 않으면서 Inline이 컴팩트 command-line 대화처럼 동작할 수 있습니다. 안정적인 항목 ID, flush 후 geometry 관찰과 일치하는 acknowledgement는 state가 관찰된 layout에서 터미널이 확인하지 않은 출력을 발행했다고 주장하지 못하게 합니다. 발행된 행을 변경할 수 없는 기본 이력으로 취급하면 resize 동작이 예측 가능해집니다. 불확실한 write에서는 정확한 터미널 stream 및 effect 진행도를 바탕으로 먼저 깨끗하게 재시작하거나 suffix만 이어서 처리할 수 있습니다. parser state와 operation 경계가 확정되어 있지만 보정할 수 없는 경우에만 사용자 소유 터미널 이력을 지우는 대신 중복 replay를 사용합니다. parser state와 operation 경계가 확정되지 않았거나 실패가 반복되면 기존 lifecycle 진단을 유지합니다.
