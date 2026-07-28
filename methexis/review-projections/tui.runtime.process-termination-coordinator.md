---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.runtime.process-termination-coordinator
revision: sha256:cbb78b65c659e052ec0683d5f9052bfedc769738dc4d94b8bcfee39f894a11e7
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:4fa668c1e12825572dbc22692dbac475bc495ee528d0663b429c67c47e5fe86d
---
# Korean Review Projection

## Translation

private `yo-cli` process host는 Unix 종료 coordinator를 정확히 하나 소유합니다. coordinator는 설치 thread에 묶인 `!Send` 값이며 초기화, 실패 설치 rollback, 모든 active-session lease, 명시적 shutdown, Drop은 설치 thread에서 실행해야 합니다. lifecycle은 `NEW -> INSTALLING -> IDLE`, `IDLE -> ACTIVE -> CLEANING -> IDLE 또는 TERMINATING`, `IDLE -> SHUTTING_DOWN -> RETIRED`이고 설치·shutdown 실패는 `FAILED_RETIRED`로 갑니다. RETIRED 계열은 다시 live가 될 수 없습니다.

설치 실패는 적용 가능한 rollback을 역순으로 모두 시도하고 primary와 모든 rollback failure를 보존합니다. exact failed-install rollback은 capture한 기존 action과 mask를 복구할 수 있습니다. 설치 handler가 관찰됐을 가능성이 한 번이라도 있으면 rollback이나 shutdown 성공 여부와 무관하게 handler가 닿는 static storage를 process lifetime 동안 유지합니다.

하나의 lock-free packed atomic word가 phase와 SIGHUP/SIGINT/SIGQUIT/SIGTERM pending bit를 가집니다. handler publication과 session finalization은 같은 word의 CAS를 linearization point로 사용합니다. `ACTIVE -> CLEANING`을 포함해 publishing phase를 벗어나는 모든 transition은 concurrent bit를 보존하는 CAS loop를 사용합니다. finalization CAS 전에 게시된 bit는 SIGHUP, SIGINT, SIGQUIT, SIGTERM 우선순위로 같은 signal을 replay합니다. 성공한 CAS가 cutoff이며 이후 handler는 IDLE/TERMINATING의 fail-closed 기본 동작을 수행합니다.

ACTIVE/CLEANING handler는 bit를 게시하고 IDLE/INSTALLING/SHUTTING_DOWN/RETIRED/FAILED_RETIRED/TERMINATING handler는 받은 signal의 기본 동작을 즉시 수행합니다. NEW는 handler가 볼 수 없습니다. host는 closure로 cleanup lease를 하나만 빌리며 `yo-tui`는 typed observation만 받아 viewport와 TTY 복구 뒤 반환합니다. finalization 전에 signal이 linearize되면 cleanup과 diagnostic 뒤 signal이 panic보다 우선하고, 이후면 원래 panic을 재개합니다.

IDLE signal은 기존 custom handler와 SIG_IGN을 덮고 즉시 기본 동작을 수행합니다. 성공적으로 초기화된 coordinator의 기존 action, flag, mask, 설치 thread caller mask는 IDLE의 성공한 shutdown만 복구하며 exact failed-install rollback만 예외입니다. partial restoration은 모든 보상을 시도하고 전체 failure를 반환하며 FAILED_RETIRED로 갑니다. Drop은 panic하지 않고 IDLE에서 shutdown을 best effort로 시도합니다. 성공해도 이미 실행 중인 옛 handler의 quiescence가 증명되지 않으므로 storage는 process lifetime 동안 남습니다. ACTIVE/CLEANING/partial failure Drop은 disposition을 일찍 복구하지 않습니다.

수용 조건은 모든 state/transition, handler publication과 `ACTIVE -> CLEANING` 및 finalization CAS 경쟁의 양쪽 결과, pending bit 보존, signal/panic cutoff, 동시 선택, idle override, 모든 installation/shutdown failure, compile-time cross-thread 이동 거부, 같은 thread mask 복구, 성공 rollback/shutdown 뒤 storage lifetime, 모든 Drop phase입니다. subprocess는 active-session 종료 전에 cleanup이 끝나며 성공 shutdown 또는 exact failed-install rollback 뒤에만 기존 action과 caller mask가 돌아옴을 증명합니다.
