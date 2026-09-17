# 런타임 후속 백로그

기준 스냅샷: 2026-09-16 (KST)

이 백로그의 원래 발견 기준은 당시 develop의 d21a457e6780 커밋이다. 아래의 기준·통합 상태는 이 스냅샷에서 확인한 사실을 보존한 역사적 기록이며, 이후 develop에 추가된 커밋의 현재 상태를 나타내지 않는다. 이 문서는 코드 커밋과 독립된 후속 판단 기록으로 관리한다. 이 문서를 갱신하기 전에는 다섯 보류 항목이었으며, 이번 갱신에서 TLS fixture 구조 정리의 완료를 별도 기록으로 보존한다. 이 문서를 갱신한 시점의 통합 develop에는 원래 기준 이후의 구조 리팩터링이 반영되어 있지만, 현재 남은 네 보류 항목의 행동 변경은 반영되어 있지 않다. main 3c7648e22528은 empty bootstrap root라 이 코드 경로가 존재하지 않는다. 이 문서는 코드 구조 리팩터링 범위를 넘어 전체 런타임 구조 개선으로 번질 수 있는 발견을 후속 판단 대상으로 남기는 기록이다. 현재 사이클에서 네 항목은 보류하며, 각 항목의 계약·검증 범위를 정한 뒤 별도 구현 작업으로 재개한다.

## 보류 항목

### 1. 명령 artifact 최종 검증과 spawn 사이의 TOCTOU

**판정:** 문서화된 호스트 한계에 대한 후속 결정이 필요한 상태.

**심각도:** 중간. 실행할 파일의 동일성에 영향을 줄 수 있는 보안·실행 경계 문제지만, 현재 계약이 이 경계를 명시적으로 보장 범위 밖에 둔다.

**근거:** crates/yo-cli/src/execution/tools/command/manifest.rs의 PreparedCommand::verify_for_launch는 frozen Artifact의 바이트와 매핑을 검증한 뒤 VerifiedCommandLaunch에 executable 경로 문자열과 인자를 넣어 반환한다. crates/yo-cli/src/execution/tools/command.rs는 그 경로로 Command를 구성하고 취소·deadline을 확인한 다음 launch.spawn을 호출한다. 따라서 마지막 검증 이후 spawn 이전에 외부 publisher가 같은 경로의 파일을 교체하면 검증한 inode와 실행한 inode가 달라질 수 있다. 현재 crates/yo-cli/src/execution/tools/command/manifest/tests.rs의 final_artifact_verification_prevents_spawn_and_blocked_stdin_is_cancellable 테스트는 실행 시작 전에 파일을 바꾸는 경우만 다룬다.

**계약:** methexis/knowledge/agent-runtime/agent.tool.local-execution-boundary.md는 frozen manifest를 사용하고 변경 시 시도를 실패시키며, 승인 후 최종 검증과 단일 spawn 직전의 취소·deadline 확인을 요구한다. 같은 문서는 primary bytes의 identity는 다루지만 uncoordinated publisher에 대한 final-check-to-path-execution race 및 hash bytes에서 원자적으로 실행하는 보장은 범위 밖이라고 명시한다. 현재 구현은 이 path semantics 계약을 따른다.

**테스트 공백:** verify_for_launch가 끝난 직후 launch.spawn 직전에 멈추는 결정적 hook 또는 barrier가 없다. 그 구간에서 publisher가 artifact를 교체해도 예상하지 않은 inode가 실행되지 않는지, 그리고 FD 고정 실행을 도입할 경우 Unix별 의미와 취소·deadline 동작이 유지되는지를 검증할 테스트가 없다.

**시도 상태:** 최종 artifact 검증과 pre-spawn 취소 확인은 원래 발견 기준 develop에 이미 통합되어 있다. 사전 변경을 다루는 회귀 테스트도 존재한다. TOCTOU를 닫는 별도 패치는 없으며, 원자적 FD 실행을 보장한다고 주장할 수 있는 구현도 없다.

**다음 결정:** 환경 한계를 계약으로 유지할지, 플랫폼별 FD 고정·pinned launch·재검증 중 하나를 설계해 계약을 개정할지 결정한다. 한계를 유지하면 가능한 범위에서 verify-to-spawn 경계의 특성화 테스트를 추가하고, 원자적 실행 보장을 암시하는 설명은 피한다. 보장을 추가하면 플랫폼 지원 범위와 실패 시도 의미를 먼저 정한 뒤 구현한다.

**main 통합 상태:** 원래 발견 기준 develop d21a457e6780에는 현재 검증 구현이 포함되어 있었다. 이 백로그에서 다루는 TOCTOU 후속 패치는 포함되지 않았으며, 이후 구조 리팩터링과도 별개다. main 3c7648e22528은 empty bootstrap root라 이 명령 실행 코드와 develop 후속 체인을 포함하지 않는다.

### 2. AgentSession terminal failure 전달 실험 ae6b9113

**판정:** 동시성 경계의 후보 수정으로 재검토가 필요한 상태.

**심각도:** 높음. terminal failure가 frontend poll과 shutdown 사이에서 유실되거나 별도 failure ledger와 terminal publication의 lock 순서 때문에 교착될 가능성을 다룬다.

**근거:** 원래 발견 기준 develop의 crates/yo-core/src/agent_session.rs는 AgentSession에 별도 failure 저장소를 두고 shutdown에서 이를 회수한다. crates/yo-core/src/agent_session/worker.rs의 ChangeLane도 failure를 별도 Mutex에 저장한 뒤 WorkerSignal::Failure를 보낸다. 반면 후보 커밋 ae6b9113은 별도 failure ledger를 제거하고 ChangeLane이 worker-owned terminal payload에 실패를 발행하게 하며, shutdown은 terminal payload에서 실패를 회수한다. 후보에는 frontend가 poll하지 않은 provider Turn failure를 shutdown이 회수하는 shutdown_retains_an_unpolled_worker_failure와 terminal 관찰·발행의 lock 경쟁이 교착하지 않는 terminal_failure_observation_and_publication_do_not_deadlock 테스트가 있다. 후보 브랜치에는 그 전 단계인 7e9be3f6과 73a80de8도 있어 terminal 전달 비차단성과 순서를 함께 다룬다.

**계약:** docs/src/architecture/code-map.md의 agent_session 경계는 nonblocking frontend access, bounded lane, worker-owned outcome, capacity-one Journal change notification, shutdown coordination을 AgentSession이 소유한다고 설명한다. 이를 기준으로 한 worker의 terminal failure는 frontend가 poll하지 않아도 shutdown에서 관찰 가능해야 하고, change가 terminal보다 먼저 전달되는 순서와 lock-order 안전성이 보존되어야 한다. frontend 독립적인 세션 의미를 core가 소유한다는 methexis/knowledge/agent-runtime/agent.core.frontend-independent-boundary.md의 원칙도 유지해야 한다.

**테스트 공백:** 원래 발견 기준 develop의 동시성 테스트는 unpolled worker cleanup failure와 receiver drop 경로를 다루지만, 후보가 추가한 unpolled provider Turn failure 회수와 terminal 관찰·발행 교착 회귀는 통합되어 있지 않다. 현재 통합 develop을 기준으로 후보 테스트를 재기반화한 뒤 agent_session 및 영향을 받는 consumer 경계 테스트를 실행해야 한다.

**시도 상태:** ae6b9113은 fix/agent-session-terminal-delivery 브랜치의 세 번째 실험 커밋이며, 후보 브랜치의 기준은 2b79c27f이다. 후보 브랜치는 원래 발견 기준 develop보다 a9f9f5fc, a13d0d86, 2056618e, d21a457e가 뒤처져 있다. 따라서 후보 자체를 현재 통합 develop에 바로 통합한 상태가 아니며, 이 백로그에서는 소스 변경을 하지 않는다.

**다음 결정:** 후보 세 커밋을 이후 develop 변경까지 반영한 현재 통합 develop에 재기반화하거나 같은 의도를 하나의 cohesive patch로 이식할지 결정한다. 먼저 failure ownership, terminal publication 순서, shutdown 회수 규칙을 리뷰하고, 결정 후 yo-core agent_session 집중 테스트와 영향을 받는 런타임 테스트를 통과시킨다.

**main 통합 상태:** ae6b9113과 그 선행 두 커밋은 원래 발견 기준 develop에 없다. 당시 develop은 후보 기준보다 앞서 있었지만 AgentSession 실험의 동작 변경을 포함하지 않았다. 이 문서 갱신 시점의 구조 리팩터링도 그 실험의 동작 변경을 포함하지 않는다. main 3c7648e22528은 empty bootstrap root라 AgentSession 코드가 존재하지 않는다.

### 3. credential 절대 경로 검증 b44cc324와 상대 YO_CONFIG 회귀

**판정:** 절대 경로 계약을 도입할 때 config 경계의 기존 상대 경로 의미를 함께 결정해야 하는 상태.

**심각도:** 높음. 상대 YO_CONFIG로 시작하는 CLI가 credential repository 생성 단계에서 실패할 수 있어 startup·connect 경로가 깨질 수 있다.

**근거:** fix/credential-root-validation의 b44cc324는 crates/yo-core/src/model_service/local_credentials/repository.rs의 LocalCredentialRepository::new를 Result 반환으로 바꾸고 storage::validate_path를 호출한다. 빈 경로와 상대 경로를 filesystem 접근 전에 거부하고, read_snapshot·lock_repository·publish에도 검증을 적용하며, rejects_empty_and_relative_credential_paths_before_filesystem_access 테스트를 추가했다. 그러나 현재 crates/yo-cli/src/state/config/tests.rs의 relative_config_filename_uses_the_current_state_directory 테스트는 YO_CONFIG=config.yaml 같은 상대 파일명을 현재 디렉터리 기준으로 지원하는 기존 동작을 고정한다. crates/yo-cli/src/state/config/path.rs는 명시된 YO_CONFIG의 상대 경로를 그대로 반환하므로 credential_path도 ./credentials.yaml이 될 수 있다. 이 값을 b44cc324의 strict constructor에 전달하면 두 경계가 충돌한다.

**계약:** methexis/knowledge/agent-runtime/agent.credentials.local-account-store.md는 repository를 nonempty absolute path로만 만들고, 빈 경로·상대 경로는 filesystem 접근 전에 실패시키며, production path는 별도로 검증된 absolute YO config root에서 파생한다고 규정한다. 동시에 현재 config 테스트와 path 구현은 명시적 상대 YO_CONFIG를 현재 작업 디렉터리 의미로 보존한다. repository 생성 전에 이 두 계약을 연결하는 정규화 또는 명시적 거부 정책이 필요하다.

**테스트 공백:** YO_CONFIG=config.yaml을 설정한 실제 CLI credential 접근 end-to-end 테스트가 없다. config 단위 테스트는 상대 경로 파생만 확인하고, b44cc324의 credential 단위 테스트는 repository path validator만 확인한다. 따라서 config load부터 credential store open까지의 회귀와 오류 메시지를 함께 검증하지 못한다.

**시도 상태:** b44cc324에 strict credential root validation과 callsite 반환 처리 변경이 존재하지만, 상대 YO_CONFIG를 정규화하거나 거부하는 후속 수정은 없다. b44cc324는 a13d0d86 위의 실험 브랜치에만 있고 develop에는 없다.

**다음 결정:** config_path 또는 load 경계에서 명시적 YO_CONFIG를 cwd 기준 absolute path로 한 번 정규화해 기존 상대 파일 의미를 보존할지, 상대 YO_CONFIG 자체를 금지하고 기존 계약·테스트·진단을 갱신할지 결정한다. strict repository에 상대 경로를 조용히 전달하는 우회는 허용하지 않는다.

**main 통합 상태:** b44cc324는 fix/credential-root-validation 브랜치에만 있다. 원래 발견 기준 develop d21a457e6780은 b44cc324와 상대 YO_CONFIG 회귀 해결을 포함하지 않았다. 이 문서 갱신 시점의 구조 리팩터링도 해당 credential 동작 변경을 포함하지 않는다. main 3c7648e22528은 empty bootstrap root라 credential 코드가 존재하지 않는다.

### 4. session 파일 NOFOLLOW 82e3df13와 root·pending·FIFO 공백

**판정:** 저장소 파일 경계 hardening 후보이며, 세 종류의 회귀 검증을 보강한 뒤 통합 여부를 결정할 상태.

**심각도:** 높음. symlink race로 다른 파일을 읽거나 FIFO에서 무기한 block할 수 있는 세션 저장소 경계 문제다.

**근거:** fix/session-file-nofollow-open의 82e3df13은 crates/yo-core/src/session_repository/local/file.rs에 pinned reader root와 openat를 도입하고, 최종 파일을 O_NOFOLLOW·O_NONBLOCK·O_CLOEXEC로 열어 regular/user-only 조건을 확인한다. append_line도 root-relative openat와 NOFOLLOW를 사용하고 pinned directory를 sync한다. crates/yo-core/src/session_repository/local/reader.rs의 read_tail_discovery와 read_snapshot_entries도 이 regular-file opener를 사용하도록 바뀌며, session log 경로의 symlink를 따르지 않는 테스트가 추가된다. 원래 발견 기준 develop은 lock file의 사전 symlink 검사와 일부 O_NOFOLLOW를 사용하지만 reader discovery/snapshot은 path 기반 blocking OpenOptions에 의존한다.

**계약:** methexis/knowledge/agent-runtime/agent.storage.session-repository.md의 local session repository 경계와 docs/src/architecture/code-map.md의 validated root·reader/file 분리 원칙에 따라 root와 user-only regular file 조건을 유지해야 한다. 후보가 구현한 descriptor pinning은 검증 후 경로 교체로 다른 tree로 이동하는 위험과 FIFO block을 줄이는 방향이다. pending marker도 같은 nofollow·regular·nonblocking 경계에서 처리되어야 한다.

**테스트 공백:** 현재 safety 테스트는 regular invalid pending marker, 빈·상대 root, writer log symlink 정도만 확인한다. reader open 뒤 root path를 교체해도 pinned tree에서 계속 읽는지 확인하는 root replacement 테스트가 없다. pending symlink와 pending FIFO가 따라가거나 block하지 않고 quarantine/error로 끝나는지 검증하지 않는다. read_tail_discovery와 read_snapshot_entries 각각에 FIFO를 둔 뒤 bounded timeout 안에 반환하는 테스트도 없다. 후보 커밋의 session log symlink 테스트만으로는 이 세 공백을 닫지 못한다.

**시도 상태:** 82e3df13에 descriptor-based NOFOLLOW·nonblocking opener와 session log symlink reader 테스트가 있다. 커밋 기준은 a13d0d86이고 원래 발견 기준 develop보다 2056618e와 d21a457e가 뒤처진 별도 브랜치다. 후보는 통합되지 않았으며 이 기록에서는 Cargo/source를 수정하지 않는다.

**다음 결정:** 후보를 이후 develop 변경까지 반영한 현재 통합 develop에 재기반화한 뒤 root pin/race, pending symlink 또는 FIFO, 두 reader 경로의 FIFO bounded test를 추가할지 결정한다. 그 검증과 yo-core session repository 집중 테스트를 통과한 뒤 플랫폼별 openat 지원 및 오류·quarantine 의미를 검토하고 통합한다.

**main 통합 상태:** 82e3df13은 fix/session-file-nofollow-open 브랜치에만 있다. 원래 발견 기준 develop에는 후보 hardening과 위의 root·pending·FIFO 보강이 없었다. 이 문서 갱신 시점의 구조 리팩터링도 해당 hardening을 포함하지 않는다. main 3c7648e22528은 empty bootstrap root라 session repository 코드가 존재하지 않는다.

## 해결된 구조 정리 기록

### 중복 TLS fixture의 소유권 경계 (W18 C28)

**처리 상태:** 해결됨. shared fixture가 OpenAI Responses의 connector-specific mode와 closed marker lifecycle을 포함한 전체 superset을 소유하고, OpenAI Responses transport lifecycle 테스트는 이를 직접 import한다.

**역사적 발견:** 원래 기준에서는 shared/yo-test-support와 OpenAI Responses가 같은 TLS child mechanics를 각각 유지했다. OpenAI Responses 사본에는 Status, EventThenStall, ErrorBodyThenStall, HeartbeatsThenStall, TlsHandshakeStall mode와 closed marker 대기가 추가되어 있었고, 두 Rust fixture와 두 Python helper가 drift할 수 있었다. docs/src/architecture/code-map.md:48의 ownership 계약은 공통 process·TLS·bounded lifecycle mechanics를 shared support에 두도록 했다.

**결과:** W18 C28 stages 1–4에서 shared fixture에 13개 mode, certificate validity checks, bounded child diagnostics, marker lifecycle을 모으고, OpenAI Responses의 local Rust fixture·Python helper·module declaration을 제거했다. OpenAI Responses에는 dev-only `yo-test-support` dependency와 직접 import만 남겼다. 원래 lifecycle 테스트는 유지됐다.

**보존된 테스트 공백:** 원래 기록에는 duplicate fixture·Python helper 재복제를 막는 CI detector와 shared/OpenAI parity test가 없다는 공백이 남아 있었다. 단일 shared owner로 drift 경로는 제거했지만 별도 duplicate-detector CI 검사는 추가하지 않았다.

**검증:** yo-test-support 12개, OpenAI Responses 46개, Kimi 19개, OpenRouter 22개, QwenCloud 15개 runtime tests가 통과했다. 현재는 shared fixture가 단일 소유 경계다.

**통합 상태:** W18 C28 stages 1–4는 `refactor/w18-shared-local-tls`에서 구현·검증됐다. main 3c7648e22528은 empty bootstrap root라는 원래 기록은 유지하며, 이 branch의 통합은 별도 절차로 진행한다.

## 완료된 범위 이탈 기록

아래 세 범위 이탈은 원래 발견 기준 checkout의 develop에 이미 완료되어 있었다. 후속 작업에서 다시 보류 항목으로 열지 않고, 관련 회귀가 발견될 때만 별도 이슈로 기록한다.

| 커밋 | 판단 | 완료 범위 | 검증·근거 | 통합 상태 |
| --- | --- | --- | --- | --- |
| a9f9f5fc | session repository root 경계 보강 | reader와 writer가 빈 경로·상대 경로를 거부하고, dot segment를 canonicalize하며, user-only root 규칙을 확인하도록 수정 | session repository safety 테스트와 절대 경로 경계 검증 | 원래 발견 기준 develop d21a457e6780에 통합되어 있었음; main 3c7648e22528은 empty bootstrap root라 해당 코드가 존재하지 않음 |
| a13d0d86 | executable helper owner 정리 | 구체적인 helper script와 validation script를 직접 CONTRIBUTING/review packet owner에게 연결하고 관련 검증을 정리 | workflow/review packet 경로와 owner 지정 변경 | 원래 발견 기준 develop d21a457e6780에 통합되어 있었음; main 3c7648e22528은 empty bootstrap root라 해당 코드가 존재하지 않음 |
| 2056618e + d21a457e | bounded-run signal cleanup 완료 | HUP·INT·TERM을 process group으로 전달하고 signal status를 보존하며, lease release 전에 descendant와 signal cleanup을 수행하고 interruption·term-ignoring 경로를 검증 | bounded-run interruption, descendant cleanup, lease cleanup 테스트 및 후속 signal cleanup 커밋 | 두 커밋 모두 원래 발견 기준 develop d21a457e6780에 통합되어 있었음; main 3c7648e22528은 empty bootstrap root라 해당 코드가 존재하지 않음 |

## 재개 순서

재개할 때는 먼저 AgentSession ae6b9113과 session NOFOLLOW 82e3df13을 이후 변경까지 반영한 현재 통합 develop에 맞춰 재기반화하고 집중 동시성·저장소 테스트를 추가한다. 그 다음 credential b44cc324의 상대 YO_CONFIG 정책을 정한 후 end-to-end 회귀를 추가한다. command TOCTOU는 보장 범위를 먼저 결정해야 한다.

이번 갱신과 함께 W18 C28의 테스트 fixture Cargo·Rust source 변경은 별도 커밋으로 수행했고 production configuration은 변경하지 않았다. 남은 보류 항목의 후속 구현은 각 항목의 다음 결정이 확정된 뒤 별도 커밋으로 수행한다.
