---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.terminal.lifecycle-restoration
revision: sha256:4403451e95ae089a621b9699d0801a6cb34300d73948a18a171ac722e06fbd19
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:50c9f6fabb06a0d182146699b17b74c6596078c79e35c21e12cc3040ce2280b3
---
# Korean Review Projection

## Translation

바깥 mode controller는 첫 변경 전에 원래 TTY input 상태를 저장하고, 각 lifecycle/mode 변경은 실제 byte나 system state가 바뀌기 전에 대응 cleanup을 등록해야 합니다. partial write나 불확실한 결과에서도 inverse action을 시도하며 일반 frame write마다 별도 보상 항목을 만들지는 않습니다. Inline과 Fullscreen은 같은 lifecycle engine을 사용하고 Fullscreen만 alternate-screen ownership을 추가합니다.

정상 종료, 진입·rendering 실패, session 경계를 넘는 panic, typed 종료 요청은 하나의 명시적 복구 경로로 모입니다. session 경계는 unwind를 잡아 terminal을 복구한 뒤 원래 unwind를 재개합니다. terminal state를 소유하는 동안 panic diagnostic은 화면에 출력하지 않고 보존하며, 복구 뒤 기존 hook을 되돌리고 diagnostic을 출력합니다. 복구는 producer 중지, synchronized update 종료/취소, style reset, 신뢰 가능한 cursor 속성 복원 또는 visible/default cursor 보장, mode 역순 해제, 소유한 alternate screen 이탈, 원래 TTY 복원 순으로 진행하며 중간 실패가 있어도 나머지를 모두 시도합니다.

Inline은 증명 가능한 viewport만 지우고 cursor를 영속 출력 아래에 둡니다. Fullscreen은 온전한 main screen으로 돌아갑니다. session은 structured outcome을 반환하고 종료 요약은 복구 뒤 caller가 출력합니다.

비동기 종료 signal은 typed control path로 들어와 terminal owner thread에서 cleanup을 수행합니다. TUI session은 같은 복구 경로를 마친 뒤에만 typed termination acknowledgment를 반환합니다. 실제 기본 disposition replay는 `yo-tui`가 아니라 process host가 같은 signal로 수행하며 숫자 exit code로 대체하지 않습니다. notification handler는 연속 session 사이에 유지되고 host의 명시적 process shutdown에서만 원래 설정으로 복구됩니다. SIGKILL, 동기 fatal fault, abort는 보장 밖입니다.

종료 관찰이 cleanup 이후 finalization 전에 linearize되면 terminal 복구 뒤 signal이 동시 panic보다 우선합니다. 보존한 panic diagnostic과 cleanup 실패는 같은 signal replay 전에 출력합니다. 그 이후라면 원래 panic을 복구 뒤 재개합니다. 명시적 복구는 primary와 모든 cleanup 실패를 함께 보고하며 Drop은 non-panicking best-effort fallback일 뿐 보고 가능한 경로를 대체하지 않습니다. job-control suspend/resume은 별도 계약 범위입니다.
