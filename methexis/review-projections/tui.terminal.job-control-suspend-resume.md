---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.terminal.job-control-suspend-resume
revision: sha256:f494d7d5c6af42bc29e68094cce97c55c2692950fa7fa42d80b6cebcea3d73a7
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:8fa5f2923661ba0951d93fbcd3268d8bbce6709f4d856a2096e7c6659b823db6
---
# Korean Review Projection

## Translation

# 작업 제어 일시 중단과 재개

## 명세

활성 터미널 세션에서 `Ctrl+Z`는 정상 종료나 termination 종료가 아니라 작업 제어 일시 중단을 요청해야 한다. 터미널을 소유한 스레드는 프로세스 host가 운영체제의 기본 suspend 동작을 수행하기 전에 터미널 이벤트 생성을 멈추고 명시적인 전체 복구 경로를 시도해야 한다. Cleanup 실패는 보고 가능하게 보존해야 하며 이후 cleanup 시도를 생략하게 해서는 안 된다.

프로세스 host는 기본 suspend 동작과 continuation 관찰을 소유해야 한다. Suspend를 숫자 exit code로 구현하거나, custom handler 아래에서 프로세스를 멈춘 채 두거나, frontend-independent application state에 Unix signal identity를 노출해서는 안 된다. 작업 제어 처리는 process termination coordinator의 termination 우선순위 및 동일 signal 재생 계약과 분리되어야 한다.

Stop 전 복구가 완료되면 process host의 현재 active cleanup lease를 닫아야 한다. 해당 lease를 finalize할 때 configured termination signal이 선택되면 termination이 우선해야 한다. Host는 기본 suspend 동작을 건너뛰고 기존 termination 계약에 따라 선택된 signal을 재생해야 한다. Termination이 선택되지 않았을 때만 coordinator가 idle phase에 도달한 뒤 host가 기본 suspend 동작을 수행할 수 있다.

Continuation 뒤에는 같은 터미널 소유 스레드가 새로운 active cleanup lease를 열고, 그 안에서 이전에 선택한 Inline 또는 Fullscreen presenter를 transaction 방식으로 다시 획득해야 한다. Application, Session, 활성 Turn, transcript, editor, pending request, scroll 상태는 terminal lease 밖에 존재하며 suspend 동안 유지되어야 한다. 재개 뒤 첫 frame은 suspend 이전 터미널 내용을 신뢰하지 않고 전체 redraw를 수행해야 한다. Inline은 새로운 viewport 소유권을 수립해야 하며 Fullscreen은 alternate-screen 소유권을 다시 획득해야 한다.

부분 mutation 뒤 재획득이 실패하거나 panic이 발생하면 등록된 모든 compensation을 시도해야 하며, live session은 불확실한 터미널 소유권으로 계속 실행하는 대신 구조화된 실패를 반환해야 한다. Suspend와 resume을 반복해도 handler, terminal mode, presenter state가 누적되지 않은 채 같은 보장을 유지해야 한다.

## 근거

Shell 작업 제어는 agent session을 끝내지 않고 터미널 소유권을 일시적으로 넘긴다. Stop 전에 복구하면 shell과 다른 foreground job을 계속 사용할 수 있다. 같은 mode를 transaction 방식으로 다시 획득하고 반드시 전체 redraw를 수행하면 `yo`가 suspend된 동안 바뀌었을 수 있는 터미널 내용을 신뢰하지 않게 된다.
