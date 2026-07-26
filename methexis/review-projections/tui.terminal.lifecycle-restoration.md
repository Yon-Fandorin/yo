---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.terminal.lifecycle-restoration
revision: sha256:f7b595fa469fd33f66f1688b02aab3099e3b450de08ee8a3dd3ebbff1c52827f
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:4b7150312f81d2aff8263ed75f011d4b7aa54f9378d0ea9892e1b9fba1635e86
---
# Korean Review Projection

## Translation

바깥 mode controller는 첫 terminal 변경 전에 원래 TTY input 상태를 저장해야 합니다. lifecycle 또는 mode 진입에 관련된 각 변경은 byte나 system state가 바뀌기 전에 대응 복구 의무를 등록하므로, partial write나 결과가 불확실한 경우에도 inverse action을 시도합니다. 일반 frame 출력마다 복구 항목을 만들지는 않습니다. Inline과 Fullscreen은 같은 lifecycle engine을 사용하고 Fullscreen만 alternate-screen 소유권을 추가합니다.

정상 종료, 진입 실패, rendering 실패, terminal session 경계를 넘어오는 panic, 처리하도록 등록한 Unix 종료 signal은 하나의 idempotent한 명시적 복구 경로로 모여야 합니다. terminal owner의 session 경계는 unwind를 잡아 복구한 뒤 원래 unwind를 다시 이어갑니다. controller는 terminal state를 소유하는 동안 panic 보고를 임시로 우회해 diagnostic metadata는 보존하되 mutable 또는 alternate screen에는 출력하지 않아야 합니다. terminal 복구가 끝난 뒤 기존 panic hook을 되돌리고 보존한 diagnostic을 출력한 다음 unwind를 재개합니다. 복구 경로는 terminal producer를 멈추고, 진행 중인 synchronized update를 끝내거나 취소하고, style을 reset합니다. cursor 속성을 신뢰성 있게 저장했다면 원래 값으로 복구하고, 그렇지 않으면 cursor를 보이게 하고 terminal 기본 모양으로 reset합니다. 이어서 등록된 input/output mode를 진입의 역순으로 해제하고, 소유한 alternate screen을 나간 뒤 마지막으로 저장한 TTY 상태를 복구합니다. 중간 복구가 실패해도 적용 가능한 나머지 복구는 모두 시도합니다.

Inline 복구는 증명 가능한 활성 viewport만 지우고 cursor를 영속 출력 바로 아래에 둡니다. Fullscreen 복구는 손상되지 않은 main screen으로 돌아갑니다. controller는 structured session outcome을 반환하며, 선택적인 종료 요약은 복구가 끝난 뒤 caller가 출력합니다.

처리하도록 등록한 비동기 종료 signal은 typed control path로 전달하고 signal handler 안에서 terminal sequence를 직접 쓰지 않습니다. 복구는 terminal owner thread에서 실행합니다. 복구가 끝나면 설치한 알림 처리를 제거하고 종료 signal을 unblock한 뒤 기본 disposition에서 같은 signal을 다시 발생시켜야 하며, 숫자 exit code로 대체해서는 안 됩니다. SIGKILL, 동기식 fatal fault, process abort는 복구 보장 밖입니다.

job-control 일시정지와 재개는 종료가 아니므로 초기 계약 범위 밖입니다. 이를 지원하려면 멈추기 전 복구와, 재개 뒤 transactional 재진입 및 전체 redraw를 별도 계약으로 정의해야 합니다.

명시적 복구는 원래 실패와 모든 cleanup 실패를 함께 보고하며 cleanup이 원인을 가리지 않게 해야 합니다. Drop은 unwind나 누락된 early return을 위한 idempotent하고 panic하지 않는 best-effort fallback만 제공하고, 보고 가능한 명시적 경로를 대체하지 않습니다.

이 계약은 이미 terminal state가 일부 바뀐 뒤 진입이 실패하거나 cleanup 자체가 실패하는 경우를 다룹니다. 사전 등록된 보상과 owner-thread shutdown은 Drop이나 signal handler 또는 단일 reverse sequence가 완전한 복구 evidence를 준다고 가정하지 않고 partial entry, raw-mode signal, panic을 처리합니다.
