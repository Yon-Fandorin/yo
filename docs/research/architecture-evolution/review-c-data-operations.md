# C 검수: R25–R36

> 일차 검수 보고서 원문이다. 검토 시점의 발견·부분 검토 범위를 보존한다.
> 이후 수정 및 최종 판정은 [전체 검수표](./review-index.md)를 기준으로 읽는다.

2026-10-05. Read-only source/design audit. 빌드·테스트 실행·네트워크·추가 agent 없음. 저장한 파일은 이 보고서뿐이다. 현재 source, 제안, 명시 보류, 미검토를 아래에서 구분한다. 이전 state verdict를 자동 승계하지 않고 각 행에서 source 또는 대응 구현의 부재를 다시 확인했다.

경로 약칭: **Y** = `/home/yon/projects/yo`; **P** = `/home/yon/projects/pi` (v1.0.2, `cd32f7725fdbddbaecdff5b1e68491563394e0ca`); **C** = `/tmp/yo-codex-rust-v0.160.0-source/openai-codex-79b1b66` (rust-v0.160.0, `a956835d020762cb2b570053af06f643a11c0ecc`). 아래 파일 경로는 해당 root 기준이다. 소스 읽기는 적힌 함수/부분과 인접 호출·test를 대상으로 했으며 파일 전부나 제품 전부를 읽었다는 뜻이 아니다.

Authority: Y `AGENTS.md`, `CONTRIBUTING.md`; `methexis/knowledge/agent-runtime/agent.core.frontend-independent-boundary.md`, `agent.observability.session-journal.md`, `agent.storage.session-repository.md`, `agent.session.continuation-lineage.md`. 제안 문서: `docs/research/architecture-evolution/{README,implementation,module-comparison,usability-comparison,review-index}.md`. 전자는 accepted 경계, 후자는 비권위 research proposal이다.

## 필요한 발견과 처리

1. **C1 / R28,R35 — 임시 첨부의 수명·전체 용량 누락. 채택 및 README 반영 확인.** 기존 후보는 파일당 upload format/size와 artifact ID만 정했다. 전송 전 취소·끊김·재시작에서 임시물 정리 owner와 host 전체 byte/count/concurrency reservation이 없었다. Parent에게 즉시 전달했고 README §8의 upload 문단(현재 853행 이후)을 다시 읽었다. 현재 self-contained normalized image snapshot은 유지하며 worker Journal commit 후 임시물을 해제한다. ACK-loss/restart는 Journal receipt와 bounded 임시 manifest로 reconcile한다. 큰 artifact DB나 외부 blob authority를 새로 요구하지 않는다. 숫자·lease 기간은 D 구현 전 고정해야 하는 명시 open parameter다. **검증은 미실행**: aggregate first excess, cancel/reject/disconnect/restart, commit 성공/ACK 유실/임시 cleanup의 조합이 필요하다.
2. **C2 / R31,R36 — durability mode와 화면의 recovery 설명 연결. Parent에게 전달 후 README §11·implementation의 Gap 표시 행·usability §7에 반영된 내용을 재검토하여 설계상 해결 확인.** Y `crates/yo-tui/src/runner/state/observation.rs:207`의 `observe_durability`는 Gap이면 `New activity stays in memory`를 고정 안내한다. 제안 service mode는 durable gap 이후 effect dispatch를 닫는다(README §8). 공통 frontend를 붙이면서 기존 문구를 그대로 쓰면 실제 가능한 행동과 다르다. 공통 projection에서 `direct memory-only`/`durable-required blocked`와 실제 가능한 inspect/recovery action을 구분하고 같은 gap의 두 모드 consumer fixture를 둔다. 기존 direct 계약 변경이나 자동 retry 요구가 아니다.
3. **C3 / R35 — 사용자 반응성과 자원 baseline이 task latency에 가려짐. Parent에게 전달 후 README §11과 usability §7의 first paint/visible acknowledgement/first model output 및 queue bytes/RSS/retained-output baseline 문구를 재검토하여 설계상 해결 확인.** 현재 eval은 total/p50/p95, 첫 올바른 사용자 행동시간, backend/사람 대기 분리를 정했다. 이것만으로 cold startup→first paint, 입력→visible acknowledgement, 큰 transcript/slow reader에서 queue·retained output·RSS 성장을 알 수 없다. U0/D acceptance에 구분된 baseline 측정 항목을 추가하면 충분하다. 임의 성능 숫자나 Pi/Codex 동등성을 선언할 필요 없다.

## 행별 근거·판단·한계

### R25 — Journal / 저장 / continuation

| 구분 | 이번에 읽은 근거와 판단 |
|---|---|
| Yo 현재 | `crates/yo-core/src/session_repository/journal.rs`의 `JournalRepository` 생성·snapshot validation·codec recovery 호출. `journal/tests/gap_recovery.rs`의 `transient_initial_read_failure_retries_with_the_complete_live_snapshot`, `capacity_gap_recovers_with_one_complete_live_snapshot`, `open_message_defers_gap_snapshot_until_its_terminal_seal` 본문. durable prefix와 live tail, open message의 terminal seal 전 snapshot 금지를 이미 다룬다. physical repository와 semantic Journal sequence를 합치지 않아야 한다. |
| Pi 현재 | `packages/coding-agent/src/core/session-manager.ts:1168`의 `_persist`/`_appendEntry`: in-memory indexes 변경 뒤 sync JSONL persist. `test/session-manager/file-operations.test.ts:47–118`: malformed line을 건너뛰고 valid entry 유지, 잘린 마지막 줄에 newline을 덧붙이는 load 경로. 이는 Yo의 검증된 exact replay/gap 계약을 그대로 대체할 근거가 아니다. |
| Codex 현재 | `codex-rs/rollout/src/recorder.rs`의 bounded writer channel·persist/flush/shutdown ACK·pending item 처리. `recorder_tests.rs:889–982` 두 본문에서 persist filesystem failure 후 buffered item retry, writer reopen 후 flush 성공을 확인한다. 별도 persistence writer/ACK 분리는 참고할 수 있지만 Yo semantic sole writer를 둘로 나누는 근거가 아니다. |
| Proposal 판단 | module §3의 repository adapter 이동은 실제 schema 변경 때 검토 후보로만 남긴 것이 적절하다. 현재 codec/storage 내부 의존을 없애려고 저장 API 전면 재설계할 필요 없음. 새로운 goal/memory도 semantic commit과 replay exact bytes 규칙 유지. |
| 검증 누락/한계 | 새 schema/upgrade/crash matrix는 아직 구현 전이다. 세 저장 엔진의 power-loss/fsync/OS filesystem 조합 전부를 검토하지 않았다. Pi의 tolerant load를 Yo에서 silent recovery로 채택하면 안 된다. C2와 연결된 service-mode gap 표시 검증 필요. |

### R26 — 장기 기억

| 구분 | 이번에 읽은 근거와 판단 |
|---|---|
| Yo 현재 | 장기 memory CRUD 구현은 아직 없다. 인접한 `crates/yo-core/src/runtime/tests/image_admission.rs:34`는 capability/budget/provenance rejection이 backend·host prepare 이전에 일어나고 같은 runtime에서 후속 정상 입력이 가능함을 검증한다. `agent_session/admission.rs:213–260`은 queue backpressure와 admission reservation의 원복 경계를 보여준다. 이를 memory snapshot admission의 기존 경계로 보존해야 한다. |
| Pi 현재 | `packages/coding-agent/src/core/resource-loader.ts`의 resource loader imports/interface와 `getAgentsFiles`/reload 축: instruction/resources 발견 책임. inspected ordinary path에서 Yo와 동등한 off/forget/CAS memory repository를 확인하지 않았다. 파일 resources/extension을 memory transaction으로 간주하지 않는다. Pi 전체 external extension 생태계는 미검토. |
| Codex 현재 | `codex-rs/ext/memories/src/backend.rs`의 private backend CRUD/search request·typed error·limit; `local/ad_hoc_note.rs`의 validated filename/nonempty/create_new/symlink 경계. `ext/memories/src/tests.rs:45–122`는 no config/disabled/dedicated tools disabled일 때 tool 미기여를 직접 검증. Codex 구현은 역할 분리 참고이며 Yo의 CAS/tombstone/off semantics와 동일하지 않다. |
| Proposal 판단 | README §7와 module §4: host가 CRUD/search/index/lock owner, core는 admitted immutable bytes snapshot owner. worker가 lock을 직접 획득하고 자기 bounded append까지 유지, 타 worker 기다리지 않기, off/forget과 동일 lock으로 stale snapshot 차단이 명시돼 있다. `/memory` 또는 GUI에서 list/inspect/edit/forget/off/scope까지 연결되어 있으며 GUI만 기다릴 필요 없다. 과거 immutable history와 미래 recall 삭제 구분도 적절. |
| 검증 누락/한계 | 실제 memory engine/UI와 tests 미구현. off/forget-before-admission, admission-before-off, CAS crash/tombstone capacity, index 손상, cross-process writer, counted=sent=recorded=resume를 구현 단계에서 검증. 기존 image test 통과를 memory 검증으로 대체할 수 없다. 자동 consolidation·global sharing은 명시 보류. |

### R27 — credentials / privacy

| 구분 | 이번에 읽은 근거와 판단 |
|---|---|
| Yo 현재 | `crates/yo-core/src/secret_store/mod.rs`의 opaque error와 encrypted store 경계; `secret_store/tests/mod.rs:289–324`의 고정 ciphertext 길이, metadata plaintext 부재, 다른 model destination 복구 거절. `request_trace.rs:1–95`는 payload-free typed observation. `application/runtime/live/presentation.rs:52–105`의 현재 resume catalog는 id/eligibility/time만 구성한다. |
| Pi 현재 | `packages/coding-agent/src/core/auth-storage.ts:1–115`의 credential file 권한/lock, `session-manager.ts:800–858`의 user/assistant text→allMessages/firstMessage catalog 추출. 인증 파일 보호와 conversation discovery 노출은 별개 축이다. Pi의 firstMessage를 그대로 복사한다고 '안전한 제목'이 입증되지 않는다. |
| Codex 현재 | `codex-rs/secrets/src/sanitizer.rs:1–110`의 특정 secret patterns와 tests; `tui/src/resume_picker_transcript_preview.rs:1–90`의 6-line/1MiB legacy scan bounds 및 thread read. `attachment-store/src/lib.rs`/`lib_tests.rs`에서 bytes와 credential-bearing file URL Debug redaction. best-effort regex는 일반 요청의 임의 개인정보 제거 보장이 아니다. |
| Proposal 판단 | SecretInput의 별도 volatile route, 제목/search/preview의 secret/private exclusion, no-payload telemetry, 명시 forget/history 차이는 보존해야 한다. U0c public catalog projection은 host가 typed allowlist로 만드는 별도 bounded read 모델이어야 하며 raw Journal serialization을 UI에 넘기면 안 된다(§8와 UX §5 결합). 일반 user prompt의 발췌는 원래 사용자 내용이다. '안전한 발췌'는 민감정보를 모두 탐지한다는 보장으로 읽히지 않게 구현 계약에서 명확히 할 것. |
| 검증 누락/한계 | 제목/search/preview 새 경로에 secret/private marker, raw provider payload, malformed records, bounded oversized text를 넣는 privacy consumer fixture가 필요. exported history의 모든 기존 경로와 모든 credential provider별 lifetime은 이번에 전수 감사하지 않았다. artifact Debug tests를 export 보안 통과로 재사용하지 않는다. |

### R28 — 첨부 / artifacts / evidence

| 구분 | 이번에 읽은 근거와 판단 |
|---|---|
| Yo 현재 | `crates/yo-cli/src/execution/image/host.rs:1–170`: host preparation 단일 job, sync_channel(1), cancellation, prepared image digest/length/display registry. `bind`가 current/inherited committed input image에서 evidence registry를 재구성하고 `Admission::validate_images`가 미준비 source를 거절한다. `runtime/tests/image_admission.rs`에서 capability/byte/host provenance rejection 검증. 현재 이미지 payload는 self-contained snapshot이다. |
| Pi 현재 | `packages/coding-agent/test/image-resize-callers.test.ts` 전체: `file-processor`/`read` 호출이 안전한 resize 불가 시 text-only omission, current model resize profile 전달, prompt dispatch까지 resizing defer를 다룬다. 이 시험이 remote host upload 소유권을 입증하지는 않는다. |
| Codex 현재 | `codex-rs/attachment-store/src/lib.rs:1–190`의 upload inline/file union, optional fresh URL TTL, redacted Debug, `lib_tests.rs` inline bytes/URL redaction tests. `app-server/src/request_processors/thread_attachments.rs:1–150`와 protocol `v2/thread_attachment.rs:1–90`: independently persisted typed metadata attachments의 bound/response-before-notify. 이 metadata attachment와 image binary store를 하나의 저장 의미로 섞지 않는다. |
| Proposal 판단 | C1 해결 방향을 재검토했고 README 새 문구가 self-contained bytes를 보존한다. host upload ID는 임시 provenance handle, Journal snapshot이 replay authority. saved evidence/current workspace/Git를 구별하는 UX §4와 충돌 없음. ref 기반 durable blob 도입은 미래 contract 변경이며 현재 요구가 아니다. |
| 검증 누락/한계 | C1 listed lifecycle tests 미구현. clipboard SSH capture 실제 플랫폼 동작과 EXIF/MIME 전체 adversarial corpus는 이번에 미실행/미전수검토. 기존 source test만으로 browser upload·current Git diff·실제 터미널 이미지가 검증됐다고 하지 않는다. |

### R29 — host / remote / reconnect

| 구분 | 이번에 읽은 근거와 판단 |
|---|---|
| Yo 현재 | `crates/yo-core/src/agent_session/admission.rs:213–260`, `startup.rs` bounded channel 생성; `crates/yo-tui/src/runner/tests/backpressure.rs` 3개 본문: full normal lane에서 exit/approval urgent response 유지, retained urgent slot이면 요청 입력을 소비하지 않음. current local ownership/token 의미는 remote DTO와 다르다. |
| Pi 현재 | `packages/coding-agent/src/modes/rpc/rpc-client.ts:1–160`, `test/rpc-client-process-exit.test.ts`: spawned process lifetime와 pending request rejection. 별도로 `src/experimental/services/connection.ts:1–125`에서 connecting/connected/disconnected와 attaching/attached/degraded 분리, exact generation hydration, binding disposal 확인. `test/experimental-remote-runtime.test.ts:1–105`는 Unix transport attach, server/client dispose, socket 0600와 directory 0700 확인. ordinary RPC와 experimental durable server를 동일 제품 경로로 주장하지 않는다. |
| Codex 현재 | `codex-rs/app-server-transport/src/connection_auth.rs` 전체: auth owner generation 변경 시 queued work invalidation. `transport/remote_control/tests/retry_tests.rs:1–90` retry-after/recovery/shutdown 과제와 auth setup; `transport/mod.rs:405–452` queue-full typed overload. `tui/src/chatwidget/reconnect.rs:1–175`: disconnect 시 media/input delivery 정지, exact message receipt reconciliation, unconfirmed 자동 재전송 금지. |
| Proposal 판단 | service가 session lifetime을 소유하고 client attachment 수명과 분리; session intent ledger와 public suffix, single controller generation, reconnect snapshot barrier, method별 완료 의미는 Pi/Codex 비교에서 보존할 실질 경계다. auth 방식은 Yo SSH-forwarded same-user first profile로 작게 유지해도 된다. |
| 검증 누락/한계 | Yo remote 구현 및 browser handshake/replay/load tests 미구현. 이번 읽기에서 Pi experimental 전체 protocol/auth/reconnect implementation과 Codex remote 전체 deployment security를 전수 검증하지 않았다. channel count bound는 total host resource bound의 증거가 아니며 attachment/client 총수 profile도 구현 전에 finite 값이 필요하다. |

### R30 — GUI / web / desktop / mobile

| 구분 | 이번에 읽은 근거와 판단 |
|---|---|
| Yo 현재 | production DOM/desktop frontend는 아직 없다. 기존 `yo-tui/src/command/help.rs` 실제 행동, `runner/tests/backpressure.rs` 지원되는 입력/urgent 처리, `runner/state/observation.rs` failure 표시가 향후 공통 journey의 현재 출발점이다. terminal renderer를 DOM renderer로 이식하자는 제안이 아니다. |
| Pi 현재 | `packages/coding-agent/src/core/export-html/template.js:1584–1645` raw HTML-like text 처리·URL renderer; `test/export-html-xss.test.ts` 전체의 scheme/control/attribute escape source assertions. 이것은 **static HTML export** 근거이며 live browser controller/ARIA/mobile의 동등 사용성을 입증하지 않는다. experimental service는 frontend-independent 접속 경계 참고다. |
| Codex 현재 | `codex-rs/app-server-protocol/src/protocol/v2/thread_attachment.rs` typed wire와 `tui/src/chatwidget/reconnect.rs` 소비자 상태 처리를 확인. pin에 Codex 상용 desktop/web의 완전한 UI source가 제공됐다는 근거는 없다. README의 app 링크는 구현 증거로 사용하지 않는다. |
| Proposal 판단 | README §9의 same journey, ordinary/choice+notes+Previous profile, DOM secret capability 미광고와 explicit compatible-controller handoff, draft revision/expiry, hostile URL/CSP/keyboard/screenreader 요구는 올바른 범위. P0 TUI 여정은 E를 기다리지 않는다. |
| 검증 누락/한계 | DOM focus trap/ARIA live region/IME/mobile touch·zoom·clipboard/assistive technology 실제 동작은 **미검토 및 미실행**. source fixture나 Pi export 검사로 통과 처리 불가. E 구현에서 같은 사용자 과제의 browser keyboard+screenreader+narrow viewport consumer tests를 별도 수행해야 한다. |

### R31 — 오류 / diagnostics / trace / recovery

| 구분 | 이번에 읽은 근거와 판단 |
|---|---|
| Yo 현재 | `runtime/error.rs` 전체의 rejected/input/backend/event/state divergence와 terminal events; `request_trace.rs` payload-free record; `session_repository/history/request_trace/tests.rs:1–90` correlated recovery fixture 구성; `yo-tui/runner/state/observation.rs:207–260` gap 원인/저장 경계 표시; `state/commands.rs:361` resume 실패 시 현재 대화 보존 안내. |
| Pi 현재 | `interactive-mode.ts:3687–3718`: auto_retry_start는 attempt/max/delay indicator와 Esc abortRetry 연결, 종료 시 handler 원복·final failure 표시. RPC child-exit test는 조용한 pending hang 대신 error 처리. |
| Codex 현재 | `tui/src/chatwidget/reconnect.rs:1–175`의 reconnect status/quit, exact confirmed ID, unconfirmed input notice, unavailable thread의 activity 해제. 전송 실패를 unsent로 단정해 재실행하지 않는 UI까지 연결한다. |
| Proposal 판단 | Retry scope가 bounded logical connector request, unknown delivery와 committed effect 재실행 금지, typed availability 및 실제 가능한 next actions는 잘 정해져 있다. C2의 구체적 UX 연결 문제는 README §11의 typed admission/recovery capability와 matrix 추가로 설계상 해결됐다. telemetry exporter failure가 업무 의미를 바꾸지 않는 owner 분리도 적절. |
| 검증 누락/한계 | 새 causal timing spans의 drop/overflow/error exporter behavior는 아직 미구현. error string만 대조하지 말고 UI action이 실제 가능하고 draft/receipt를 보존하는 consumer tests 필요. request trace fixture 앞부분만 읽었고 전체 serializer roundtrip test body를 전수 읽은 것은 아니다. |

### R32 — tests / eval / CI

| 구분 | 이번에 읽은 근거와 판단 |
|---|---|
| Yo 현재 | `.github/workflows/unix-compile.yml` 전체: workflow_dispatch-only Linux/macOS cargo check+clippy. `tools/validation/yo-cli-unix-matrix.sh` 전체는 current-host compile과 다른 OS unverified 표시. `crates/yo-cli/src/pty_tests/termination.rs` 전체는 real PTY SIGTERM 후 termios/fullscreen 복원과 signal replay. 단위/PTY 자산은 있으나 현재 workflow가 PR end-to-end 품질 gate라는 주장은 불가. |
| Pi 현재 | `.github/workflows/ci.yml` 전체: PR build/check/test와 별도 pinned MCP conformance; `.github/workflows/build-binaries.yml:131–215`: 실제 package archive extract 후 help/version/codemode binary smoke. 기존 기능 tests와 실제 package consumer를 분리한다. |
| Codex 현재 | `.github/workflows/blocking-ci.yml` 전체와 `.github/scripts/check_ci_results.py` 전체: version-controlled needs, always fan-in, success 이외 fail. 이름만 required인 job이나 skipped dependency를 pass로 두지 않는다. transport/rollout tests 위 행에서 직접 확인. |
| Proposal 판단 | implementation verification matrix와 eval manifest(source/model/profile/fixture/hash/budget/oracle, missing/unscored exclusion)는 좋은 개선안. PR gate와 eval 품질, actual package consumer와 workspace compile을 명확히 분리한다. tests 수가 UX 동등성 증거가 아니며 이미 문서가 이를 구분한다. |
| 검증 누락/한계 | 어떤 CI/test도 실행하지 않았다. paid/model task oracle benchmark와 UI 실제 task 측정 미실행. expected job manifest를 future implementation에서 관리해야 하며 조건부 legitimately skipped와 unexpected skip을 구분하는 테스트 필요. C1–C3 matrix 연결 후 새 acceptance 구현 필요. |

### R33 — install / upgrade / rollback / compatibility

| 구분 | 이번에 읽은 근거와 판단 |
|---|---|
| Yo 현재 | README 첫 110행의 현재 제품/기능/개발 preview 설명, `.github/workflows/unix-compile.yml`, Unix compile helper. 정식 packaged install-upgrade-resume consumer 증거는 이번 sampled source에서 확인하지 못했다. 아직 proposal인 것과 current official command를 혼합하지 않는다. |
| Pi 현재 | root `README.md:1–100` install→directory→pi→/login 진입과 pinned install/update 설명. release workflow의 extracted binary smoke와 packed npm consumer 단계(346행)가 제품 설치 경로에 연결된다. |
| Codex 현재 | root `README.md:1–105` install/launch/login; `scripts/install/install.sh:1–120` version validation, package root/current link/install lock, bounded download timeout; `scripts/install/test_install_sh.py:134–158` verified package installation request fixture. installer 테스트는 모델 entitlement나 upgrade history compatibility를 자동 입증하지 않는다. |
| Proposal 판단 | README §11 manual verified package replacement, state 보존, incompatible schema mutation 전 거절, compatible rollback, outside-workspace SDK smoke는 과도한 updater 확장 없이 적절. 자동 updater는 H 이후 보류 유지. Yo Unix 범위 때문에 Windows feature parity를 끌어올 필요 없다. |
| 검증 누락/한계 | 실제 Linux/macOS clean install·upgrade·rollback 및 schema migration 아직 미실행. 모든 Codex installer branch/OS signing/rollback tests를 읽지는 않았다. future schema가 이전 package에서 읽기 불가하면 'compatible package'가 무엇인지 release manifest에 밝혀야 한다. mutation 후 binary만 되돌려 데이터도 복구됐다고 하면 안 된다. |

### R34 — help / docs / workflow

| 구분 | 이번에 읽은 근거와 판단 |
|---|---|
| Yo 현재 | `README.md:1–110` 긴 기능/preview 설명이 quickstart 전에 위치. `crates/yo-tui/src/command/help.rs` 전체는 실제 Alt+D/Output/queue/request 선택과 capability별 명령을 기술. `CONTRIBUTING.md:1–100` ordinary task에 formal Slice 강제 없음, task 승인 지속, 관련 contract/검증/독립 review와 push 별도. |
| Pi 현재 | root README의 짧은 install→cwd→pi→/login; SDK/RPC/extension 링크 분리. ordinary user 진입에 internal package/module diagram을 요구하지 않는다. |
| Codex 현재 | root README의 install→codex→login과 docs 분리, installer `--help`의 actual grammar. 사용자가 처음에 알아야 할 선택부터 안내한다. |
| Proposal 판단 | usability §2의 검증된 quickstart 우선, preview/developer docs 별도, help의 actual behavior/capability 기반 힌트는 근거에 맞다. README §12에서 non-authoritative proposal을 바로 accepted source로 취급하지 않으며 contract delta 때 SOT owner를 갱신해야 한다. 반복 확인을 process에서 제거하되 product permission scope 변경은 실제 승인 경계로 남기는 것도 일치. |
| 검증 누락/한계 | source README 재구성은 아직 제안이다. 새 `yo resume`/`yo serve`/attach grammar를 current 도움말로 넣으면 안 된다. 문서 link 검사만으로 명령 실행 가능·no-config 진입·capability hints를 검증하지 못한다. Pi/Codex contributor authority 전체는 이번 행에서 재검토하지 않았다(사용자 도움말/Yo workflow가 범위). |

### R35 — latency / capacity / resources

| 구분 | 이번에 읽은 근거와 판단 |
|---|---|
| Yo 현재 | `yo-tui/src/runner/unix/timing.rs:1–145` motion deadline/idle no poll/retry interval, `timing/output_timing.rs:1–110` 실제 ANSI sink에 simulated I/O cost를 넣는 deterministic test. `agent_session/admission.rs` urgent lane/backpressure, `runner/tests/backpressure.rs` exit/approval 유지. 이미 응답성과 queue 경계 자산이 있어 전면 scheduler 교체를 요구하지 않는다. |
| Pi 현재 | `packages/tui/src/tui.ts:990–1048` coalesced 16ms scheduling과 immediate input frame preemption. `test/render-churn-bench.ts:1–95` allocation churn/wall-time, null terminal의 범위를 명시한다. Pi benchmark도 terminal I/O나 retained RSS를 측정한 것처럼 해석하면 안 된다. RPC client의 stderr string 누적은 Yo에 그대로 복제할 패턴이 아니다. |
| Codex 현재 | `app-server-transport/src/transport/mod.rs` CHANNEL_CAPACITY=128과 queue-full typed overload test(405행), rollout bounded channel, bounded resume preview scan. per-channel count bounds와 entire RSS budget은 다른 주장이다. |
| Proposal 판단 | scheduler 4-slot window/raw reservations/encoded budget, per-attachment control queue bounds, first-excess matrix는 구체적이다. C1 aggregate temporary upload profile 추가는 필요한 최소 보완. C3의 first-visible feedback와 long-output/slow-consumer baseline을 더하면 사용자 체감과 자원 설계가 연결된다. |
| 검증 누락/한계 | 실제 cold start, UI p95, long-session RSS/heap/disk, fanout slow-reader stress 미실행. schema/count cap이 RSS 상한 증거는 아니다. terminal sink unit test를 real SSH/tmux 프레임 지연 측정으로 제시하지 않는다. host client/attachment aggregate limits는 D 시작 전 finite profile로 고정해야 한다. |

### R36 — 연결 및 검수 누락 자체 점검

| 교차 여정 | 판정 / 남은 확인 |
|---|---|
| setup→input→attachment→admission→Journal→resume | Y host image evidence reconstruction/runtime rejection, Pi resize caller, Codex inline/file store를 직접 비교했다. C1 수정은 현 snapshot authority를 보존하며 끊김/cleanup의 빠진 owner를 메운다. upload numeric profile은 명시 열린 항목이며 구현 전에 결정한다. |
| source error→durability policy→client action | Y observe_durability 고정 memory-only 문구와 research durable service fail-closed 사이 C2를 발견했다. 같은 Journal gap이 mode에 따라 실행 가능한 행동을 달리하므로 projection+UI consumer test로 묶어야 한다. |
| private secret→catalog/search→remote preview/export | encrypted secret store만 확인하고 privacy 전체를 pass로 하면 누락이다. R27에서 ordinary text 발췌와 protected payload exclusion, public projection owner를 분리했고 new catalog/export 검증 미실행을 표시했다. |
| memory off/forget→worker snapshot→restart | host CRUD와 worker immutable bytes commit의 권한이 구분되어 있다. `/memory`가 TUI에서도 제공될 제안이며 GUI 종속은 없다. race와 transaction recovery는 아직 구현 테스트 과제. history purge를 memory forget과 섞지 않는다. |
| queued input→disconnect→reconcile→resume | Pi ordinary RPC와 experimental service를 구분했고 Codex exact-confirmed input flow를 직접 읽었다. Yo remote intent/state proposal은 그 목적에 맞다. unknown을 자동 resend하지 않는 consumer 검증은 아직 없다. |
| package→quickstart→first action→task evidence | R32–R34에서 actual workflow/installer/help를 읽었다. source compilation·mock task·실제 auth·package 설치는 서로 다른 증거. future package+external consumer gate는 구현 전이다. |
| output growth→frame/transport pressure→cancel | Y deterministic output/urgent tests, Pi scheduling/churn benchmark, Codex overload test를 읽었다. current eval total latency만으로 first feedback/RSS 개선은 입증 못 하므로 C3. |

검수 범위 자체의 한계: R25–R36 전 행에 직접 source 근거와 proposal 판단을 배치했다. 다른 담당의 R01–R24 전체 원본을 재감사하지 않았으며 이 보고서는 그 행들의 통과를 대신하지 않는다. 제품 전체 보안 감사·Pi 외부 extensions·상용 Codex web/desktop source·실제 provider/OS/SSH/DOM conformance는 수행하지 않았다. 발견이 없는 행도 새 기능 구현 완료로 보지 않는다. 최종 fresh-context 검수는 위 C1–C3 수정의 구현 전제와 테스트 미실행 상태, R27 public catalog privacy fixture, R30 명시 미검토 영역을 빠뜨리지 않아야 한다.

## 최종 권고

현재 연구안을 뒤집을 구조 결함은 찾지 않았다. C1–C3 모두 parent의 최소 문서 수정 후 해당 문단과 verification matrix/사용 과제를 다시 읽어 설계상 해결을 확인했다. 원발견은 위에 보존했다. 구현·수용 테스트는 별도이며 이번 감사에서 실행하지 않았다. 이 보고서의 통과 범위는 source/design 검토이며 구현·수용 테스트·성능·상용 제품 동등성 판정이 아니다.
