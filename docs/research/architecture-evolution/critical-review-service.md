# 서비스·운영 공격적 재검수 — R25–R36

> Status: non-authoritative research/design audit

2026-10-05, static read-only audit. 사용자 요청은 “전부 공격적으로 검토해줘, yo가 부족한 서비스라고 생각해”다. 기존 보고서의 “설계상 해결”, “남은 material finding 없음”을 서비스 수용 판정으로 승계하지 않는다. 이번에는 **처음 설치한 사람이 작업을 끝내고, 다시 찾고, 장애를 복구하고, 안전하게 업데이트·삭제할 수 있는가**를 판단 기준으로 삼았다.

제품·테스트·계약은 변경하지 않았다. 빌드·테스트·네트워크·모델·하위 agent 실행 없음. 이 파일만 작성했다. 아래 테스트는 읽은 기존 증거이며 이번 실행 결과가 아니다. 리모트·브라우저·실제 설치·성능의 미검증을 숨기지 않는다.

경로: **Y** `/home/yon/projects/yo`; **P** `/home/yon/projects/pi`, 지정 pin `cd32f7725fdbddbaecdff5b1e68491563394e0ca`; **C** `/tmp/yo-codex-rust-v0.160.0-source/openai-codex-79b1b66`, 지정 pin `a956835d020762cb2b570053af06f643a11c0ecc`. 외부 경로는 로컬에서 직접 읽었으며 release provenance/network는 재확인하지 않았다.

읽은 authority: Y AGENTS.md, CONTRIBUTING.md, README.md. 기존 연구 `review-index.md`, `review-c-data-operations.md`, `implementation.md`, 연구 README의 memory/host/GUI/operations 및 관련 rollout. 기존 TUI 콘텐츠 검수에서 직접 확인한 Changes/Output/copy 경계도 사용했다.

## 판정

**Yo는 저장·상관관계·권한 경계를 세심하게 구현한 개발 중 인터페이스이지만, 현재 저장소에서 확인 가능한 배포·운영·복구·실사용 증거만으로 완성된 서비스를 주장하기 어렵다.** 어려운 내부 경계를 구현한 점은 보존해야 하지만, 사용자가 할 수 없는 기본 업무를 면제하지 않는다. 가장 큰 문제는 기능 개수가 아니라 **제품을 얻는 경로, 장애에서 내용을 꺼내는 경로, 오래 쓴 데이터를 관리하는 경로가 끝까지 연결되지 않은 것**이다.

기존 계획은 우선순위를 더 공격적으로 바꿔야 한다. `G`에 함께 들어간 provider/MCP/timing/evals/install release는 하나의 출시 단계가 아니다. 문서상 “서로 독립”이라고 덧붙이는 것만으로 출시 기능의 owner·완료 조건이 생기지 않는다. 배포·복구·데이터 수명과 CI를 **지금 존재하는 TUI의 선행 서비스 작업 S0**로 끌어올리고, remote/browser는 최소 하나의 재접속 여정을 별도로 완주해야 한다.

## R25–R36 coverage

| 항목 | 현재 판정 | 실제 확인 경계 | 가장 큰 사용자 결손 / 연결 finding |
|---|---|---|---|
| R25 저장/continuation | **있지만 부족** | `yo-core/src/journal/tests/gap_recovery.rs`, `session_repository/{journal.rs,local/repository.rs,local/file.rs}` | exact prefix/complete snapshot은 있으나 사용자 공간 관리·live 구조·복구 지시가 끊김. S3/S4 |
| R26 memory | **없음** | `Cargo.toml`, CLI `Command`, 연구 §7; Codex memory 실제 backend/tests 비교 | 새 대화가 검증한 학습을 가져오는 UI·engine 없음. 계획의 F3를 구현으로 평가할 수 없음. S6 |
| R27 credentials/privacy | **있지만 부족** | `secret_store/{mod.rs,tests/mod.rs}`, `session_repository/local/wire.rs`, CLI session grammar | 비밀 저장 보호와 일반 대화·첨부의 retention/deletion/export 통제가 별개. S4 |
| R28 attachments/artifacts | **있지만 부족 / 일부 노출 안 됨** | `yo-cli/src/execution/image/host.rs`, tests; TUI Changes/Output | self-contained image·saved diff는 있으나 작업 결과 목록·가져가기·버전/수명 관리가 제품 여정으로 통합되지 않음. S4/S8 |
| R29 host/remote/reconnect | **없음(새 service profile)** | `application/runtime/live.rs`, `yo-core/src/host.rs`, CLI grammar; Pi experimental connection, Codex reconnect | SSH terminal 실행을 durable service attach/reconnect로 취급 불가. S7 |
| R30 GUI/web/desktop/mobile | **없음 / 실사용 미검증** | workspace members·CLI, 연구 §8–9 | production DOM frontend 없음. 설계 matrix나 Pi HTML export는 브라우저 제어 UX 증거 아님. S7 |
| R31 diagnostics/recovery | **있지만 부족** | `interaction/diagnostic.rs`, `application/codex_diagnostics.rs`, TUI `observe_durability` | 오류 문구·trace는 있으나 모델 호출 없이 자신의 설치/저장/호환 상태를 진단하고 실제 복구할 단일 경로 부족. S3/S5/S9 |
| R32 tests/eval/CI | **있지만 출시 gate 부족 / 실사용 미검증** | `.github/workflows/unix-compile.yml` 유일 workflow, PTY test, ignored live tests | 자동 PR test gate 및 실제 package 여정 증거 없음. 테스트 수가 대신하지 못함. S2 |
| R33 install/update/compatibility | **없음 또는 부족** | `yo-cli/Cargo.toml`, workflow, Codex initialize allowlists | version 0.0.0/publish false, package/update/rollback route·호환 profile UX 없음. S1/S5 |
| R34 docs/help | **있지만 진입 설계 부족** | 700여 줄 README·실제 CLI grammar·help | 최초 설치/실행 안내보다 기능 세부·preview가 앞서고 사용자 guide와 developer source 설명 혼재. S1 |
| R35 latency/capacity/performance | **경계 자산 있음 / 실측 미검증** | `local/repository.rs::storage_bytes/append`, timing/PTY 경계, image worker | count/byte cap은 p95/RSS/startup SLA가 아님. 관측해야 할 구체 비용 경로 존재. S9 |
| R36 completeness/누락 | **설계 검토와 제품 완료 혼동 위험** | implementation stages, review-index statuses, current files | “설계상 해결”을 모든 기본 journey의 완료처럼 읽게 하는 위험. S0 release scoreboard 필요. S2/S8/S9 |

## S1 — 설치·업데이트·문서가 제품 진입로를 제공하지 않는다

**분류: 없음/있지만 부족. 우선순위: P0 서비스 출시 선행.**

- **현재 source:** [Y: crates/yo-cli/Cargo.toml](../../../crates/yo-cli/Cargo.toml)은 `version = "0.0.0"`, `publish = false`; binary 이름 `yo`. 공개 배포 자체가 없다는 외부 단정은 하지 않지만, repository에는 실제 package 배포 workflow가 없고 `.github/workflows/unix-compile.yml` 하나만 있다. `README.md`는 시작부터 상세 `/new`/resume/fork, 52행 개발용 preview, 각종 chart/image/extension 설명으로 이어지며 명시적인 일반 사용자 install→connect→first task quickstart가 확인되지 않는다. cargo preview 예제는 일반 사용자의 설치 방법이 아니다.
- **비교:** P README는 installer/npm, 필요 런타임, cwd→pi→`/login`, update 경로를 앞에 둔다. C README는 installer/npm/Homebrew/플랫폼 archive→codex→sign-in을 앞에 둔다. 이 명령들은 이 감사에서 실행하지 않았고 보안성/성공률은 별도다. 비교 포인트는 **명백한 최초 행동이 제공된다**는 것이다.
- **기존안 반박:** “first upgrade manual; self-updater H”는 합리적이지만 **검증 가능한 package·지원 플랫폼·실패 복귀**까지 후속으로 미룰 이유가 아니다. auto-update를 만들지 않는 것과 배포 경험을 만들지 않는 것은 다르다.
- **사용자 before→target:** 소스/README를 탐색하고 어떤 binary·host 버전을 써야 하는지 추측 → 지원 플랫폼의 검증된 package를 받고 `yo --version`/read-only preflight 확인, 한 연결 생성, 첫 작업 후 저장 재개, 다음 package로 교체하고 실패 시 이전 호환 package로 복귀.
- **선택:** **도입**: release manifest+checksums+플랫폼 archive와 실제 package smoke. **재설계**: root README 상단은 1-screen quickstart, 지원/미지원과 저장 위치/복구 링크; 기능 reference·preview/renderer API는 별도 문서. **보존**: manual upgrade 허용, no forced daemon. **삭제**: 설치 이전에 내부 renderer/chart 설정을 읽어야 하는 문서 순서. 대안은 임시 source-install 경로 하나를 명시적으로 지원하고 동일 clean-environment smoke를 붙이는 것; 여러 installer ecosystem 동시 도입은 불필요.
- **Owner·선행:** CLI/release owner + product docs; SDK 분리/MCP/GUI/child/memory 불필요. versioning/support policy 작은 결정 필요.
- **수락:** Linux/macOS의 clean state root와 repo 밖 cwd에서 **배포 artifact** 설치→help/version→no-config setup→mock/local fixture 첫 턴→종료/재개. old→new state upgrade와 incompatible package 거절, 기존 데이터 mutation 없음, documented rollback 성공. 실제 auth는 명시된 live smoke로 별도 기록. command docs를 실제 parser/실행 결과와 대조.

## S2 — 테스트 자산은 많지만 자동 회귀 방어와 서비스 성공 증거가 없다

**분류: CI gate 없음 / 실제 여정 미검증. 우선순위: P0.**

- **현재 source:** 유일 Y workflow는 `workflow_dispatch`만 trigger하며 Ubuntu/macOS에서 `cargo check -p yo-cli --all-targets`와 workspace clippy만 한다. PR/push test, runtime test, package install/upgrade job 없음. `tools/validation/yo-cli-unix-matrix.sh`도 compile 후 다른 OS를 unverified로 정직하게 표시한다. `crates/yo-cli/src/pty_tests/termination.rs::fullscreen_termination_restores_real_pty_before_signal_replay`는 **실제 PTY** 복구 시험이 있어 이를 “모든 테스트가 mock”이라고 부르면 틀린다. 그러나 authenticated `crates/backends/delegated-codex/tests/live_agent_session.rs::{local_codex_completes_a_real_file_change,local_codex_resumes_a_durable_session_and_remembers_prior_input}`는 ignored/별도 실행이다.
- **비교:** [P: .github/workflows/ci.yml](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/.github/workflows/ci.yml)는 PR build/check/test와 MCP conformance, `build-binaries.yml`는 배포 archive 추출 후 binary smoke. C `blocking-ci.yml`와 `.github/scripts/check_ci_results.py`는 dependencies의 success만 허용하고 skipped/cancelled도 실패 처리한다. GitHub의 실제 branch protection 설정은 로컬 source로 확인하지 않았으므로 required enforcement가 이미 운영된다고 단정하지 않는다.
- **기존안 반박:** 현재 1,000개 이상의 특정 suite 통과가 있어도 “새 사용자가 설치하고 일을 끝내는가”의 oracle가 아니다. implementation A에 affected CI를 묶고 G에 eval/package를 넣은 구조는 release readiness를 기능 rollout 부록으로 만든다. 지금 PR gate부터 분리 실행해야 한다.
- **before→target:** 개발자가 수동으로 관련 suite를 골라 실행하고 문서상 검토 완료 → 모든 후보가 동일 최소 gate를 통과하며 package와 사용자 task의 실패를 자동으로 걸러냄.
- **선택:** **도입**: 작은 자동 PR gate, always aggregate, 배포물 offline journey smoke, versioned task fixture/outcome manifest. **보존**: 기존 단위/PTY tests, 비용 있는 live eval 분리. **삭제**: 테스트 개수·link 검사·review 완료를 제품 완성의 대리 지표로 쓰는 상태 표시. 대안: 전체 workspace를 매번 돌리기보다 변화 영향+필수 핵심 journey, 정기 전체 suite; omission이 fail-open 되지 않도록 required selection oracle 유지.
- **Owner·선행:** validation/release + 각 consumer; 원격이나 모델 vendor 확장 선행 없음.
- **수락:** intentional failing/skipped required job이 aggregate를 실패시킴; package smoke가 누락 assets/잘못된 version을 검출; bugfix task는 diff와 test oracle로 완료 판정; unknown usage/skipped live eval은 성공이나 0 비용으로 집계하지 않음. macOS/Linux/SSH/tmux의 실행 여부·버전·결과를 명시. paid model eval은 별도 승인된 budget만 사용.

## S3 — 저장 실패를 알아도 사용자가 중요한 내용을 구조할 수 없다

**분류: 기능 있음, 복구 여정 부족. 우선순위: P0.**

- **현재 source:** [Y: yo-tui/src/runner/state/observation.rs::observe_durability](../../../crates/yo-tui/src/runner/state/observation.rs)는 `History not saved`, saved cutoff, `New activity stays in memory. Copy important output before closing yo.`를 표시한다. 그런데 `runner/state/commands.rs::CopyAnswer`는 **최신 완료 assistant answer 하나**만 전송하고 nontext/oversize/미완료를 거절한다. 기존 Output는 retained tool source reader이며 전체 live tail 구조/export 동작이 아니다. `yo session`은 `command/session.rs::run`의 **stored reader**이므로 아직 저장되지 않은 live tail을 구할 수 없다.
- **보존할 실제 강점:** `yo-core/src/journal/tests/gap_recovery.rs::{capacity_gap_recovers_with_one_complete_live_snapshot,open_message_defers_gap_snapshot_until_its_terminal_seal,integrity_gap_stops_automatic_snapshot_retries}`는 prefix/terminal seal/손상에서 자동 재시도 중지를 검증한다. `session_repository/local/file.rs::append_line`은 sync/rollback/pending marker를 갖는다. 재실행/자동 repair를 추가하자는 뜻이 아니다.
- **기존안 반박:** C2를 “direct/service 문구 분리로 설계상 해결”한 것은 의미 정확성만 해결했다. **현재 direct mode의 실제 save-rescue 행동은 여전히 없다.** 실패 복구에는 문구와 typed cause 외에 사용자 데이터가 나오는 출구가 필요하다.
- **비교:** P `agent-session.ts::{exportToHtml,exportToJsonl}`·interactive export 경로는 일반 사용자용 content evacuation 선례다. C reconnect는 입력을 복구 큐로 보존하고 confirmed submission ID와 reconcile한다. 둘 다 Yo exact-resume archive의 대체는 아니지만 실패 시 내용을 버리지 않게 하는 사용자 행동을 제공한다.
- **before→target:** 저장 안 됨 경고 후 최신 답 하나를 복사하거나 화면을 수작업 캡처 → 현재 live public conversation/tool evidence·draft를 안전한 목적지로 구조하고 무엇이 저장/미저장/재개 불가인지 확인; 공간 복구 후 안전한 재저장 결과 확인.
- **선택:** **도입**: user-authorized read-only rescue export, metadata preview/size/destination, secret/private exclusion, live/partial provenance, 저장 가능한 다른 경로 선택. **보존**: durable prefix 불변, integrity gap 자동 snapshot 중지. **금지/삭제**: “export되었으니 exact resume 가능” 암시, unknown tool effect 자동 재실행. 대안은 full portable archive보다 작게 bounded readable rescue document부터; 잘림은 명시하고 pagination/chunk 선택 제공.
- **Owner·선행:** live Journal projection owner + CLI/TUI export effect; remote host 없어도 구현 가능. export privacy 정책과 live-tail identity 필요.
- **수락:** 저장 용량/권한/무결성 fault 각각에서 여러 assistant/tool/user/미완료 데이터 보존 범위를 사용자가 확인; draft/첨부가 지워지지 않음; export 실패를 성공이라 표시하지 않음; raw secret/provider-private bytes 없음; 복구 파일이 원래 repository를 수정하지 않음; success 후 실제 내용 검증.

## S4 — 저장 공간과 개인정보 수명 관리가 제품 기능으로 빠져 있다

**분류: 없음(일반 Session lifecycle UI), 보호 경계는 있음. 우선순위: P0/P1.**

- **현재 source:** [Y: state/storage/environment.rs](../../../crates/yo-cli/src/state/storage/environment.rs) 기본 repository cap은 **1 GiB**. `session_repository/local/repository.rs::append`는 root-wide byte cap을 검사하고 초과 시 pressure. CLI `Command`와 `command/session.rs::Arguments`는 조회/표시만 제공하며 일반 Session archive/delete/retention/backup/restore grammar 없음. `secret_store/tests/mod.rs::{save_replace_restart_and_delete,fixed_ciphertext_and_no_public_plaintext}`는 **secret store**에 적용되며 일반 대화 삭제나 모든 기록 암호화를 증명하지 않는다. `local/wire.rs::WireEntry`는 JSON payload string+checksum 저장 경계다; checksum은 encryption이 아니다.
- **기존안 반박:** 연구 memory §7은 forget은 미래 recall만 제거하고 history/backup은 별도라고 올바르게 구분하지만 **history purge를 별도 기능이라고 선언한 뒤 owner·일정·수락이 없다.** 오래 사용하는 서비스에서는 capacity cap과 수명 관리가 한 기능이다. “삭제가 복잡하다”는 reason은 scope preview/안전한 archive부터 시작할 근거지 무기한 부재의 근거가 아니다.
- **비교:** P session selector는 `deleteSessionFile`와 confirmation/error를 실제 제공하고 trash→unlink fallback이 있다. 이 단순 파일 삭제를 Yo fork/secret/lineage에 그대로 복사하면 안 된다. C의 secret sanitizer나 attachment Debug redaction은 일반 데이터 lifecycle 기능의 증거가 아니다.
- **before→target:** 저장소가 찼을 때 env cap을 올리거나 내부 파일을 수동 삭제 → 공간을 어떤 Session/첨부가 쓰는지 보고, active/lineage 의존성을 확인하며 archive/export/delete scope를 선택; 백업·이미 제출된 remote 데이터·자식에 복제된 history는 무엇이 남는지 분명히 표시.
- **선택:** **도입**: storage usage inspector, retained-data inventory, explicit archive/delete plan+실행, backup/restore 검증 경로. **보존**: 활성 writer locks, exact replay integrity, secrets 별도 암호화. **재설계**: session list를 단순 resume picker가 아니라 관리 경로와 연결. 대안은 first release에서 active Session 삭제 금지·불명확 lineage read-only preview·archive 우선; 이것도 사용 가능한 reclaim 결과가 있어야 함.
- **Owner·선행:** repository lifecycle/domain owner + CLI/UI; schema delta가 필요하면 authority 갱신. memory/GUI 선행 불필요.
- **수락:** 공간 reclaim 뒤 새 append 재개, concurrent writer와 delete 경쟁 실패 원자성, parent 삭제 후 self-contained child의 표시/복구 의미, secret 저장소와 ordinary history의 삭제 범위 분리, backup restore 때 host/account/workspace 검증, 실패 시 original 보존. “사용자 제어 삭제”와 storage secure erase 보장은 구별.

## S5 — 호스트 호환성은 코드에 있으나 설치·선택 UX와 배포 정책으로 연결되지 않았다

**분류: 있음/노출 부족/실사용 미검증. 우선순위: P0 지원 profile 확정, P1 UX.**

- **현재 source:** [Y: backends/delegated-codex/src/protocol/initialize.rs](../../../crates/backends/delegated-codex/src/protocol/initialize.rs) 일반 verified minor는 `[145,146,149]`; 다른 0.x는 warning 후 계속한다. `image_wire_version_supported`는 정확히 `0.153.4|0.154.0`만 허용. `tests/live_agent_session.rs` secret test는 0.155.1을 요구하며 ignored다. 이번 비교 source pin은 0.160.0이다. `protocol/tests.rs::{warns_for_an_unverified_codex_minor_line,grants_image_wire_policy_only_to_exact_reviewed_patches}`는 이 분리 정책을 검증한다. **이 자체가 bug거나 모든 newer minor가 unsafe라는 주장은 아니다.** 사용자가 이해할 검증 profile이 부재한 것이 문제다.
- **before→target:** 최신 Codex를 설치한 뒤 warning/이미지 거절/기능별 지원을 뒤늦게 발견 → 현재 binary와 계정·모델 capability에 대해 text/image/approval/question/secret/resume의 supported/experimental/unsupported를 작업 전 읽고 검증된 설치 profile을 선택.
- **기존안 반박:** 연구의 “Codex compatibility fixture 필요”만으로 고객 지원 가능한 version policy가 되지 않는다. 최신 소스와 비교했으므로 어댑터도 최신이라고 오해하게 만드는 표시는 제거해야 한다. 모든 0.x 차단은 비용 큰 대안이고 현재 계약을 무심히 변경한다; 기능별 검증 구분을 사용자 친화적으로 노출하는 것이 최소다.
- **선택:** **도입**: package와 함께 version/capability evidence manifest, read-only preflight, verified host installation instructions와 recheck. **보존**: exact image gate, unknown capability의 명확한 제한, warning observer boundedness. **재설계**: “지원” 하나의 불리언 대신 기능별 상태. 대안은 제한된 host version 하나의 tested support lane + 명시 experimental lane.
- **Owner·선행:** delegated adapter/release/setup owner; real paid/live probe는 별도 budget·credential 범위 필요.
- **수락:** 알려진/미확인 버전과 모델 조합에서 기능 안내가 실제 admission 결과와 일치; external version drift 후 새 probe 및 저장 Session resume failure에 되돌릴 행동 제공; supported claim은 실행한 named fixture와 exact binary 버전에 연결. source pin을 live adapter conformance 결과로 사용하지 않음.

## S6 — 메모리는 설계가 아니라 사용자가 써볼 수 있는 작은 기능으로 먼저 나와야 한다

**분류: 없음. 우선순위: P1 (S0와 독립).**

- **현재 source:** workspace/CLI 및 core 후보 검색에서 user-facing memory CRUD/recall service 구현 없음; 연구 §7의 `MemoryRepository`, `MemorySelectionSnapshot`, `/memory`는 제안이다. 기존 image admission, CAS, durable input 원리는 재사용 기반이지 memory 통과 증거가 아니다.
- **비교:** [C: ext/memories/src/local/ad_hoc_note.rs::add_ad_hoc_note](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/ext/memories/src/local/ad_hoc_note.rs)는 validated filename/nonempty/create_new 실제 note path, `ext/memories/src/tests.rs::tools_are_not_contributed_when_disabled` 등은 contribution capability를 검증한다. P resource loader/AGENTS/extension을 곧바로 off/forget/CAS memory라고 부르지 않는다.
- **기존안 반박:** F3 independent라고 한 판단은 맞다. 그러나 off/forget lock·자동 contribution·stale·derived index를 한 덩어리로 완성할 때까지 “다음 대화에 배운 것을 기억”을 제공하지 않는다면 기능 규모가 사용자 가치를 막는다.
- **before→target:** 같은 프로젝트 새 대화마다 build caveat와 확정 선호를 다시 설명 → 사용자가 승인한 workspace note를 저장/조회/수정하고, 다음 요청에 실제 포함된 내용을 보고 끄거나 잊게 함.
- **선택:** **도입**: explicit user-owned scoped notes + 선택된 recall snapshot의 최소 vertical slice. **보존**: counted=sent=recorded=resume, off/forget-before-admission 차단, 권한으로 memory 해석 금지. **후속**: 자동 추출/자동 consolidation/vector search/global sharing. **삭제**: 기술적으로 거대한 “학습 시스템”을 최초 memory 완료 조건으로 묶는 전제. 파일 저장도 충분하나 동시성·scope·snapshot 계약은 생략하지 않음.
- **Owner·선행:** memory storage/host adapter + core admission + TUI; GUI/F1/F2 불필요.
- **수락:** note를 사용자에게 보여준 뒤 저장; 새 대화 actual request bytes와 replay에 동일 note; 다른 workspace 차단; off/forget race와 crash에서 resurrection 없음; memory 미지원 호스트에 성공처럼 보이지 않음; note가 tool permission 확대 못함.

## S7 — 리모트·브라우저는 중요한 서비스 격차이며 설계 표로 덮을 수 없다

**분류: 없음(신규 service/frontend). 우선순위: P1 명확한 최소 vertical, 기존 TUI S0와 독립.**

- **현재 source:** [Y: application/runtime/live.rs::run_live_session](../../../crates/yo-cli/src/application/runtime/live.rs)은 현재 프로세스의 config/termination/resource lifecycle을 소유한다. `yo-core/src/host.rs`는 `WorkspaceHostId`·workspace identity이지 attachable server가 아니다. CLI `Command`에 serve/attach 없음, workspace에 production DOM/desktop consumer 없음. tmux/SSH를 통해 현재 TUI를 유지하는 운영 방법은 유용하지만 client-detach/host-lifetime/receipt-reconcile API를 제공한 것과 다르다.
- **비교:** [P: experimental/services/connection.ts](https://github.com/earendil-works/pi/blob/cd32f7725fdbddbaecdff5b1e68491563394e0ca/packages/coding-agent/src/experimental/services/connection.ts)는 connection과 attachment 상태 및 exact generation hydration을 분리한다. 이름 그대로 experimental이며 일반 Pi RPC가 동일 durable service라고 과장하지 않는다. [C: tui/src/chatwidget/reconnect.rs::{pause_for_disconnect,restore_reconnected_input}](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/chatwidget/reconnect.rs)는 input delivery 중지, media cleanup, confirmed message ID reconcile, uncertain input 복구 경로를 구현한다. Codex 상용 GUI 내부는 이 source 감사 대상 아님.
- **기존안 반박:** D/E가 인증·ledger·snapshot·모든 질문·artifact·browser·desktop까지 묶이면 큰 구현이 끝나기 전 사용자가 경험할 결과가 없다. “안전 경계가 많아서 아직 설계 중”은 완료 정의를 작게 자를 이유다. first DOM secret 미지원은 가능하지만 그 대신 **실제 사용 가능한 controller handoff**를 완성해야 한다.
- **before→target:** 연결이 끊기면 프로세스/terminal 복구 방법을 사용자가 관리 → 같은 호스트에서 실행이 유지되고 재접속 후 정확한 Session/turn/approval 상태로 돌아와 중복 제출 없이 계속.
- **선택:** **도입/재설계**: D0 local stdio/Unix trusted profile의 독립 client attach vertical → D1 authenticated SSH-forwarded browser 단일 화면 vertical → desktop thin shell 순서. S0 TUI 배포/복구는 이 작업을 기다리지 않음. **보존**: opt-in host, no forced daemon, single controller generation, typed unknown, no auto resend. **삭제**: browser와 desktop 동시 제공을 첫 service completion 조건으로 묶는 범위.
- **Owner·선행:** host session factory/service + protocol + 한 consumer; browser 단계부터 Origin/auth/CSRF/limits 필수. mutation 지원 전에 durable intent/receipt semantics 필요하되 모든 미래 feature 지원까지 필요하지 않음.
- **수락:** packaged client 두 개로 한 Session 관찰/명시 transfer; 승인 중 controller 종료→재접속→동일 request 응답; submit ACK loss→정확히 1회 실행; unknown은 명시; slow/disconnected observer가 controller cancel 막지 않음. DOM keyboard/IME/screenreader와 실제 SSH route는 별도 실제 실행 기록.

## S8 — 첨부를 넣는 기능은 있지만 작업 결과를 가져가는 서비스는 얇다

**분류: 있음/노출 안 됨/여정 부족. 우선순위: P1.**

- **현재 source:** [Y: execution/image/host.rs::{bind,Admission::validate_images}](../../../crates/yo-cli/src/execution/image/host.rs)는 준비한 이미지 evidence를 current/inherited committed input에서 복구하고 단일 preparation lane을 유지한다. tests `worker_evidence_is_required_before_skill_preparation`, `cancellation_releases_the_single_preparation_lane`는 유효하다. TUI `runner/view/{changes,output}.rs`는 이미 retained diff/large tool output을 읽게 한다. 이것을 “artifact 기능 전무”라고 하면 틀린다. 그러나 CLI session grammar는 Chat/Transcript/Request 조회이고 task 결과 파일·reported artifact·검증 결과·retained source를 한곳에서 찾아 export/open하는 product surface는 확인되지 않았다.
- **기존안 반박:** upload cap/cleanup은 input lifecycle 문제다. 사용자에게 중요한 것은 **작업이 만든 무엇을 어디서 확인하고 다음 도구로 가져가는가**다. self-contained image snapshot과 complete diff evidence 설계만으로 output delivery가 완료되지 않는다.
- **비교:** P HTML/JSONL export는 대화를 가져가는 실제 경로다. [C: history_cell/patches.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/tui/src/history_cell/patches.rs)/transcript source-copy는 patch inspection/copy를 연결한다. 이 둘을 general artifact repository와 동일시하지 않지만 채택할 최소 UX 선례다.
- **before→target:** 완료 메시지를 읽고 여러 tool을 뒤져 경로·출력·diff·검증을 찾음 → 작업 완료에서 changed files/result artifacts/tests를 source-qualified 목록으로 열고, 현재 filesystem과 retained evidence를 구분하여 필요한 결과를 내보냄.
- **선택:** **도입**: 작은 Session result index/projection와 기존 Changes/Output의 contextual jump/export. **보존**: 출력이 report인지 actual captured bytes인지 구분; before-image 없으면 unavailable; no auto fetch/path trust. **후속**: 범용 blob DB, 자동 revert, 외부 cloud artifact service. 대안은 existing local file outputs+retained text/diff만 범위로 먼저 제공.
- **Owner·선행:** public result projection + CLI/TUI; remote upload보다 먼저 가능. artifact ID/provenance와 action capability 최소 계약 필요.
- **수락:** old completed task의 generated report/diff/test evidence를 기억한 내용으로 찾기; file 변경/삭제 뒤 stored evidence vs current file 명확; unsupported open은 이유; export content 일치; redacted/private payload 제외; 실패 결과를 성공 deliverable로 포함하지 않음.

## S9 — 운영 진단과 성능 개선의 출발점이 측정값 대신 계획에 머문다

**분류: 있음/노출 부족/실측 미검증. 우선순위: P1 baseline과 preflight, 병목 수정은 측정 뒤.**

- **현재 source:** [Y: interaction/diagnostic.rs::AppError::{with_help,render}](../../../crates/yo-cli/src/interaction/diagnostic.rs)에는 context/cause/tip 출력, `CodexWarningCollector`에는 bounded warning과 dedup가 있다. payload-free trace도 보존할 자산이다. CLI grammar에는 종합 read-only health/preflight/support-report 경로가 없다. 구체 측정 후보: `session_repository/local/repository.rs::append`가 root coordinator/lock 안에서 `storage_bytes()`로 모든 root entry의 metadata를 scan하고, `local/file.rs::append_line`은 sync/marker cleanup을 수행한다. 따라서 Session 수·filesystem 특성에 따른 commit 대기 비용을 **측정해야 한다**. 이 코드만으로 현재 느리다거나 데이터 유실한다고 단정하지 않는다.
- **비교:** P tui scheduling/churn benchmark는 renderer 내부 비용 비교 선례지만 remote I/O/RSS 서비스 보장의 대체 아님. C bounded transport/rollout/channel도 전체 host RSS/SLO 보장이 아니다. upstream의 숫자를 Yo target으로 무비판 채택하지 않음.
- **기존안 반박:** 연구 C3가 first-paint/ACK/RSS를 문서에 추가했다고 “해결”은 아니다. 서비스가 자주 멈춘다고 느끼는 사용자는 모델·승인·저장·tool·terminal 중 누가 기다리게 하는지 알아야 한다. 내부 causal spans는 user next-action까지 연결되어야 한다.
- **before→target:** warning 문자열과 환경변수를 보고 원인을 추측 → 모델 호출 없이 실행 version/host capability/state-root health/capacity/terminal mode를 확인; 작업 중 저장 대기·모델 대기·approval·cancel 상태를 구별; export 가능한 bounded support report에서 원인을 재현.
- **선택:** **도입**: read-only preflight + redacted support report, offline realistic-volume baseline. **보존**: no-exporter default, prompt/secret 미포함, telemetry failure 업무 의미 분리. **재설계 후보**: 측정으로 root scan 비용이 의미 있을 때 bounded/accounted index·storage accounting owner 개선; sync durability를 속도 때문에 제거하지 않음. **삭제**: “Rust/가벼운 모듈/빠른 단위 시험이므로 충분히 빠름” 추정.
- **Owner·선행:** CLI diagnostics + repository/TUI timing owners; GUI/MCP 선행 없음. numeric budgets·target hardware/terminal 조건은 baseline 직전 고정.
- **수락:** 1/100/1,000 saved sessions, realistic text/image/tool volume에서 cold first paint, submit visible ACK, append/lock wait, first model output 별도 기록; slow sink/SSH/tmux cancel response; process RSS·disk growth와 retained bounds; 최소 regression budget을 결과에 맞춰 설정. redacted report에 no prompt/secret/raw tool output, malformed labels 안전, failure isolation. 성능 run을 이번 감사가 실행했다고 표시하지 않음.

## 바꿀 도입 순서와 완료 표기

1. **S0 즉시 독립 서비스 spine:** package/version/support profile, quickstart, 자동 PR gate, 실제 artifact smoke, 저장 Gap 구조/export, storage inventory와 안전한 수명 관리. 현재 TUI만으로 수락 가능하다. provider/MCP/GUI architecture 완료를 기다리지 않는다.
2. **독립 P1 value:** minimal explicit memory, result index/exports, contextual recovery/preflight. 기존 core ownership과 exact snapshot을 유지한 작은 사용자 여정 단위로 구현한다.
3. **Remote vertical:** 독립 trusted client/host부터 정확한 disconnect/reconnect/control transfer를 완주한 뒤 browser, 마지막 desktop wrapper. UI framework 이전에 사용자 과제가 먼저다.
4. **Evidence scoreboard:** 각 항목에 `없음`, `구현 있음`, `노출 부족`, `deterministic 검증`, `실제 platform/host 검증`, `제품 task 성공`을 별도 칼럼으로 둔다. source review 완료/설계상 해결을 뒤 칼럼에 복사하지 않는다. 설치 package/CLI help/지원 matrix/실제 tests 중 하나라도 서로 모순되면 출시 완료로 부르지 않는다.

이 순서는 기존 엄격한 저장·권한·비밀 경계를 느슨하게 하자는 제안이 아니다. **사용자가 서비스를 믿을 수 있게 하는 가장 기본적인 행동을 추상 설계보다 먼저 끝내자는 우선순위 변경**이다. runtime 측정 없이 성능 열세, 미공개 Codex GUI source, Pi 전체 extension 생태계 동등성은 주장하지 않는다.

## 정정과 한계

- 이전 C 보고서의 gap test 경로는 `session_repository/journal/tests/gap_recovery.rs`로 읽힐 수 있으나 현재 실제 경로는 `crates/yo-core/src/journal/tests/gap_recovery.rs`다.
- ignored PTY child test는 부모 real-PTY test가 의도적으로 실행하는 helper이다. 이를 ignored authenticated live eval과 같은 “미실행 제품 시험”으로 집계하면 안 된다.
- `publish=false`는 crates.io 비공개 표시이지 외부 어떤 배포물도 존재하지 않는다는 증거가 아니다. 이번 판정은 이 저장소의 실제 release/install pipeline와 문서 경로에 한정한다.
- 세금·법적 privacy compliance를 평가한 것이 아니다. data deletion/backup/export는 일반적인 사용자 서비스 과제로 평가했다.
- no builds/network에 따라 이 보고서의 latency·release install·auth·SSH/tmux·browser task 결과는 모두 **실행 대기**다. 적힌 실패는 코드로 확인한 missing pathway/semantic mismatch이며, 발생률이나 실제 생산 장애 횟수가 아니다.

통합 판단과 변경된 배달 순서는 [전체 비판 검수](./critical-review.md)를 따른다.
