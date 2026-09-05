# 검증

변경된 경계를 기준으로 증거를 고른다. 기대 동작과 중요한 실패를 구분할
수 있는 가장 작은 검사부터 시작한다. 그다음 검사를 넓히고
[Slice](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#slice-contract)를
닫는다.

## 증거 계층

| 계층 | 확인할 수 있는 것 | 예시 |
|---|---|---|
| 프로세스 내부 | 결정론적 상태, protocol, layout, rendering, 주입된 실패 동작 | `yo-core` engine/runtime test, `yo-tui` component test, rendering parity golden |
| 호스트 통합 | 선택 설치 프로그램 없이 실제 호스트 기능을 사용한 동작 | `yo-cli`의 Linux PTY, termios, process signal, 터미널 복원 test |
| 외부 환경 | 설치 프로그램, 인증, 중첩된 터미널 환경과의 호환성 | Codex, Grok, tmux, 로컬 `sshd`, SSH, SSH 내부 tmux |

첫 번째 계층은 빠르게 진단할 수 있지만 OS 터미널 생명주기를 증명하지
못한다. 호스트 통합 계층은 실제 Unix 경계를 실행하지만 모든 터미널
multiplexer나 원격 session을 증명하지 못한다. 외부 환경 계층은 실제로
실행한 환경에 대해서만 그 빈틈을 채운다.

무시되었거나 실행할 수 없는 환경 검사는 passed가 아니라
**unverified**다. assertion을 약하게 만들거나 조용히 건너뛰지 말고 빠진
command, host, credential, platform을 기록한다.

## 변경된 경계에서 시작하기

| 변경 영역 | 처음 실행할 유용한 명령 | 가장 가까운 증거 |
|---|---|---|
| Session, Turn, Activity, engine, runtime 의미 | `cargo test -p yo-core` | `crates/yo-core/src/tests`와 소유 모듈 test |
| typed input span, submission identity, 고정된 v1의 structured-reference 거절 | `cargo test -p yo-core input::tests`와 `cargo test -p yo-core journal::codec` | `crates/yo-core/src/input/tests.rs`와 Journal wire-compatibility test |
| Agent-session admission, concurrency, 시작, 종료 | `cargo test -p yo-core agent_session::tests` | `crates/yo-core/src/agent_session/tests` |
| backend lifecycle, evidence 또는 bounded child-process transport 추출 | `cargo test --locked -p yo-backend` 뒤 `cargo test --locked -p yo-core backend::evidence`와 `cargo test --locked -p yo-core journal::codec::tests::correlation` | `crates/backends/foundation/src`, `yo-core` specialization, Journal wire·recovery 호환성 test |
| Codex protocol 변환이나 provider ID 연결 | `cargo test --locked -p yo-backend-delegated-codex` | `crates/backends/delegated-codex/src/tests.rs` |
| Grok ACP 변환, permission, 인증, Session 연결 | `cargo test --locked -p yo-backend-delegated-grok` | `crates/backends/delegated-grok/src/tests.rs`와 `protocol.rs` |
| 해석된 input, 편집, paste, binding, 종료 gesture | `cargo test -p yo-tui input::` | `yo-tui/src/input` 곁의 test |
| prompt 줄 바꿈, cursor 표시, viewport | `cargo test -p yo-tui prompt::` | `yo-tui/src/prompt` 곁의 test |
| `@` trigger, stale 결과, 선택 치환, local 순위, Git ignore 탐색 | `cargo test -p yo-tui workspace_reference`와 `cargo test -p yo-core workspace_reference` | `yo-tui/src/prompt/workspace_reference.rs`와 `yo-core/src/workspace_reference` |
| `$` trigger, Codex catalog decode, scope filtering, 비활성 행, typed skill 선택 | `cargo test -p yo-tui skill_reference`, `cargo test -p yo-core skill_reference`, `cargo test -p yo-backend-delegated-codex skill_catalog` | `yo-tui/src/prompt/skill_reference`, `yo-core/src/skill_reference`, `backends/delegated-codex/src/skill_catalog.rs` |
| 대화 기록 item, streaming revision, scroll | `cargo test -p yo-tui transcript::` | `yo-tui/src/transcript` 곁의 test |
| shell 조합, layout, Surface, Unicode 너비, text flow | `cargo test -p yo-tui` | 소유 `yo-tui` 모듈 곁의 test |
| ANSI operation이나 표시 mode 정책 | `cargo test -p yo-tui terminal::` | `yo-tui/src/terminal` 아래 test |
| Inline 또는 Fullscreen mode 동작 | `cargo test -p yo-tui terminal::mode::` | `yo-tui/src/terminal/mode` 아래 test |
| live loop 순서, backpressure, submission draft 소유권, event Projection | `cargo test -p yo-tui runner::` | `yo-tui/src/runner` 아래 test |
| 같은 완성 frame의 터미널·HTML Projection | `cargo test -p yo-tui --test rendering_parity` | `crates/yo-tui/tests/rendering_parity`와 golden |
| process termination이나 실제 터미널 복원 | `cargo test -p yo-cli pty_tests::` | `crates/yo-cli/src/pty_tests/` |
| Unix process coordinator 상태와 보상 | `cargo test -p yo-cli execution::process::termination::tests` | `crates/yo-cli/src/execution/process/termination/tests` |
| 공통 bounded YAML parse·inference·failure budget | `cargo test -p yo-yaml` | `shared/yo-yaml/src/lib.rs` |
| Rust test 바로 위에 필요한 설명 | `cargo xtask check test-explanations` | `crates/`, `shared/`, `tools/` 아래 Rust source |
| Slice 변경이 bind된 로컬 write-set 안에 머무는지 | `cargo xtask check slice-scope` | 하나의 활성 Slice worktree; planner가 먼저 `cargo xtask slice-contract bind <contract.json>` 실행 |
| 두 Slice contract의 현재 통합 기준점이 같고 선언한 소유권이 겹치지 않는지 | `cargo xtask check slice-parallel <left.json> <right.json>` | direct Slice는 `develop`, Wave Slice는 해당 Wave branch 사용 |
| 하나의 깨끗한 Slice 후보에서 검증, 리뷰, 위험, 승인 증거가 모두 같은 identity에 결속됐는지 | `cargo xtask slice gate <request.json>` | 검증이나 리뷰를 다시 실행하지 않고 다음 행동 하나만 반환 |
| ready Slice의 정확한 commit message와 close 기록을 identity 전사 없이 준비하는지 | `cargo xtask slice commit prepare <gate.json> <message-source> <message-out>` 실행 후 exact squash를 commit하고, `close plan/apply` 전에 `cargo xtask slice close prepare <request.json>` 실행 | 첫 prepare는 깨끗한 Slice worktree, close prepare는 accepted commit 이후 깨끗한 통합 worktree에서 실행 |
| 저장소 hook 정책이나 구조화된 개발 검사 | `cargo test -p xtask` | `tools/xtask/src` |
| Prospective activation ContextBuild와 review-packet identity | `cargo test -p methexis activation_review_context`와 `cargo test -p xtask review_packet::tests::prospective` | 정확한 activation request, 제안 Checkpoint·active record, authority mode, packet 재생, active-authority 교차 사용 거절 |
| tmux, SSH, SSH 내부 tmux 동작 | [터미널 환경 매트릭스](./terminal-matrix.md) 참고 | ignored `yo-cli` 환경 test |

이 명령들은 시작점이지, 영향받은 인접 경계를 무시해도 된다는 허가가
아니다. 예를 들어 `AgentSession` 수정으로 frontend가 보는 admission
결과가 달라진다면 집중 test와 TUI runner test가 모두 필요할 수 있다.

model-connector request와 stream 검증은 변경한 dialect를 소유하는 concrete Connector
crate(예: `cargo test --locked -p yo-connector-openai-chat-completions` 또는
`cargo test --locked -p yo-connector-kimi`)를 실행하고, 공용 byte
lifecycle mechanics가 영향받으면 `cargo test --locked -p yo-connector-transport`도 함께
실행한다. 종료할 때는 중립 어휘와 managed-loop consumer를 위해
`cargo test --locked -p yo-core`를 실행한다. 환경 통합 Connector 검사는 로컬
`127.0.0.1` HTTPS listener만 사용하며 ephemeral test certificate를 만들고
serve하기 위해 외부 `python3`와 `openssl` 명령을 요구한다. 필수 조건이
없으면 assertion을 skip하지 않고 명령이 실패한다. 각 validation 실행마다
host/platform, prerequisite version, passed/unverified 결과를 기록한다.

## 결과 읽기

- **Passed**: 적어둔 명령이 해당 환경에서 assertion을 성공적으로 실행했다.
- **Failed**: 명령이 실행되어 mismatch, timeout, panic, cleanup error를
  발견했다. 처음 실패한 소유 경계를 따라가고 뒤이은 cleanup failure도
  보존한다.
- **Unverified**: 필요한 환경에서 검사가 실행되지 않았다. coverage gap으로
  계속 보이게 둔다.

golden과 snapshot은 fixture의 정확한 Projection을 증명한다. 의도적으로
갱신할 때는 diff를 검토한다. 다시 생성했다는 사실만으로 새 출력이
올바르다고 판단하지 않는다.

## 에이전트에 반환하는 출력 제한하기

자세한 검증 출력이 에이전트 context로 돌아갈 때는
`tools/validation/bounded-run.sh`로 실행한다. wrapper는 command의 exit status와
합쳐진 전체 출력을 보존하고 worktree-local
`.local-exclude/validation-runs/` 디렉터리에 둔다. 성공하면 JSON summary 한 줄만
반환한다. 실패하면 같은 summary와 마지막 diagnostic output 최대 16 KiB를 반환한다.
그 tail만으로 소유 실패를 찾을 수 없을 때만 전체 local log를 확인한다.

기본 summary schema는 frozen `yo.validation-run-summary/v1alpha2`이다. 실행을 시작할 때의
`HEAD`, worktree가 clean이었는지, 정확한 command 인자 개수와 경계를 구분하는 hash,
전체 log의 byte 수와 SHA-256, `reviewed-descendant/v1` 재사용 정책을 기록한다.
따라서 Slice gate는 clean 후보의 결과를
선언된 command와 자체 결속된 evidence로 비교할 수 있다. dirty summary는 local
진단에는 쓸 수 있지만 후보 evidence로는 쓸 수 없다. summary는 실제 실행을
기록하므로 항상 `"reused":false`이며 이전 실행을 자동 탐색하거나 재사용하지 않는다.
후속 gate는 동일한 정확 command의 통과 summary이고 trusted Git이 clean 실행 HEAD를
검토된 최종 후보의 조상으로 증명할 때만 `"reused":true`를 선언할 수 있다. frozen
`yo.validation-run-summary/v1`과 `v1alpha1` artifact는 원래 의미로 gate 호환성을
유지하며 v1alpha1은 재사용을 허용하지 않는다.

결과가 local 저장소 byte만으로 결정되는 command에는 `--reusable-local`을 추가한다.
이 opt-in은 `reviewed-descendant-context/v1` 정책을 가진
`yo.validation-run-summary/v1alpha3`을 출력한다. v1alpha2 결속에 더해 target OS,
architecture, Rust/Cargo toolchain fingerprint를 기록한다. 후속 reused gate에서 Yo는
이 값을 다시 관측하고 달라졌으면 fail-closed한다. `external_state:"none-declared"`
선언은 network, clock, account, service 또는 그 밖의 external state에 의존하는
command를 제외한다. 그런 command는 재실행한다. 이 옵션은 이전 receipt를 탐색하지
않으며 기존 summary를 변경하지 않는다.

stdout을 복사하지 않고 review와 gate preparation에 쓸 summary를 보존하려면 ignored
부모 디렉터리를 만들고 직접 발행한다.

```bash
mkdir -p .local-exclude/coordination/<slice>/validation
bash tools/validation/bounded-run.sh \
  --summary-out .local-exclude/coordination/<slice>/validation/workspace-tests.json \
  --reusable-local \
  workspace-tests -- cargo test --workspace --all-targets
```

output file과 stdout 한 줄은 byte-identical하다. 발행은 atomic create-only다. 부모가
없거나 target이 이미 있으면 validation command 전에 중단하고, 동시에 생긴 target도
덮어쓰지 않는다. 발행한 파일을 immutable review packet에 추가하면 manifest가 경로와
hash를 `slice gate prepare`에 제공한다. 이는 새 evidence 저장만 수행하며 이전 결과를
재사용하지 않는다. 재사용 판단은 이 runner가 아니라 후속 검토가 끝난 Slice gate
request가 소유한다.

wrapper는 표시만 바꾸고 검증 의미는 바꾸지 않는다. log는 임시 운영 artifact다.
필요한 실패 log는 finding이 미해결인 동안만 보존하고, 완료한 log는 Slice
worktree와 함께 폐기한다.

## 한 후보의 게이트 통합하기

Slice 후보가 깨끗한 commit이 되면 bounded validation JSON summary와 각 최종
review 응답을 별도 local file로 저장한다. 정확한 hash, 후보 commit, canonical diff
hash, 필수 lens, 알려진 미검증 환경, 위험 분류, human-origin 승인을
`yo.slice-gate-request/v1alpha1` request에 기록한 뒤 다음을 실행한다.

```bash
cargo xtask slice gate /tmp/<slice>-gate.json
```

선언한 검사 하나와 완료한 lens 하나가 있는 최소 request 형태는 다음과 같다.
검사나 lens가 더 필요하면 해당 evidence entry를 반복한다.

```json
{
  "schema": "yo.slice-gate-request/v1alpha1",
  "candidate_commit": "<full-commit>",
  "required_lenses": ["fresh-context"],
  "validation_evidence": [{
    "name": "workspace-tests",
    "argv": ["cargo", "test", "--workspace", "--all-targets"],
    "result_path": "/tmp/workspace-tests.json",
    "result_hash": "sha256:<summary-hash>",
    "candidate_commit": "<full-commit>",
    "reused": false
  }],
  "review_evidence": [{
    "lens": "fresh-context",
    "reviewer": "provider/session",
    "route": "model-high/provider/model/session",
    "verdict": "clear",
    "candidate_commit": "<full-commit>",
    "diff_hash": "sha256:<canonical-diff-hash>",
    "result_path": "/tmp/fresh-context.txt",
    "result_hash": "sha256:<response-hash>"
  }],
  "known_unverified_environments": [],
  "risk": {
    "classification": "human-attention",
    "rationale": "changes workflow authority"
  },
  "approval": null
}
```

사람이 exact approval을 완료하면 `null`을 `kind: "exact_candidate"`,
`human/<identity>` authority와 scope, 같은 `candidate_commit`과 `diff_hash`로
교체한다. routine request는 human-origin scope가 이 작업을 포함하고 미검증 환경이
남지 않았을 때만 `kind: "standing_routine"`을 사용하고 두 exact identity field를
생략할 수 있다.

이 command는 결속된 Slice 범위, 깨끗한 `HEAD`, 경로에서 도출한 최소 lens,
evidence file hash, 후보/diff identity, review route, approval 형태를 확인한다.
그리고 `validate`, `review`, `approve`, `integrate` 중 정확히 하나를
`next_action`으로 담은 `yo.slice-gate-result/v1alpha1` JSON 한 줄을 반환한다.
그 행동 자체를 실행하지는 않는다. 후보 변경, stale diff, 변조된 evidence file,
경로 기반 lens 누락은 다음 행동을 만들지 않고 fail-closed된다.

이는 증거 일관성 검사이지 선언이 참이라는 증명은 아니다. validation plan의
완전성, semantic review lens, 위험 분류, 기록한 verdict의 정확성은 여전히
coordinator가 소유한다. request와 evidence는 ignored coordination storage 또는
worktree 밖에 두고 Slice 종료 시 제거한다.

## Slice 종료 기준선

집중 검사가 통과하면 저장소 기준선을 실행한다.

```bash
bash tools/validation/bounded-run.sh workspace-tests -- cargo test --workspace --all-targets
bash tools/validation/bounded-run.sh workspace-clippy -- cargo clippy --workspace --all-targets -- -D warnings
bash tools/validation/bounded-run.sh hk-candidate -- \
  hk check --check --from-ref BASE_SHA --to-ref CANDIDATE_SHA
```

`cargo test`는 일반 test를 실행하고 ignored test를 compile하지만, 환경
의존 ignored test를 실행하지 않는다. `hk check`는 변경 경로에 따라
`hk.pkl`에서 저장소 검사를 고른다. formatting, test 설명, 영향받은
crate 검사, Methexis 검사, Developer Docs 검사가 여기에 포함된다.
설치와 hook 사용법은
[`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#local-checks)가
소유한다.

staged Methexis 변경이 `methexis/sources/`와 `methexis/knowledge/` 경로로만
이루어지면 hook은 먼저 작업 중인 Methexis tree가 index와 정확히 같고 추적되지
않은 Methexis 경로가 없는지 확인한 다음 `records`와 `relations` class만 실행한다.
따라서 이전 Projection이 의도적으로 stale인 semantic-first 후보를 commit할 수 있다.
staged Projection, approval, Checkpoint, active record 또는 그 밖의 Methexis 경로가
하나라도 있으면 authority를 포함한 완전 validation 경로를 그대로 사용한다.

편집 중에는 로컬 Slice contract에 선언한 집중 검사를 사용하고, 결과가
완성되면 이 Slice 종료 기준선을 한 번 실행한다. 정확한 Methexis activation
후보가 staged된 구간에는 `hk`가 prospective validation을 사용하고 일반
Methexis test를 잠시 미룬다. 통합 직후에는 trusted `develop`에서 일반 전체
Methexis check와 test를 실행한다.

해당 activation worktree는 clean `develop`에서
`cargo xtask slice create-activation <request.json>`으로 준비한다. 생성된
contract는 active record, Checkpoint tree, 등록된 context manifest 두 개를
lease한다. 집중 검사인 `methexis check --staged-activation`은 새 immutable
Checkpoint를 정확히 하나만 허용한다. Slice 생성은 coordination setup일 뿐
prospective transition이 유효하다는 증거가 아니다.

이후 독립 activation을 검수할 때는 이를 지원하는 workflow 구현이 이미 trusted인
경우에만 명시적인 v1alpha3 review request를 쓴다. 위 집중 test는 trusted-capability
bootstrap, 정확한 activation-only 경로 경계, proposal identity와 canonical packet 재생을
확인하지만, 후보는 통합 전 staged activation 검증과 통합 직후
일반 전체 Methexis 검증을 여전히 거쳐야 한다.

Slice가 platform이나 외부 환경 경계를 바꾼다면 기준선이 이를 검사했다고
주장하지 말고 관련 matrix 명령을 추가한다.

검수한 후보를 squash했다는 이유만으로 바뀌지 않은 기준선을 다시 실행하지
않는다. 새 fast acceptance는 integration HEAD가 candidate base이고,
`hk check --check --from-ref BASE_SHA --to-ref CANDIDATE_SHA`의 두 SHA가 gate의
정확한 base와 candidate이며, 결과가 성공·비재사용·외부 상태 없음으로 기록됐고 OS, architecture,
Rust/Cargo fingerprint가 여전히 일치할 때만 candidate-bound 결과로 중복 commit
hook을 대신할 수 있다. 이 선택은 `candidate_hk_receipt`로 기록한다. 그 밖에는
`git_hooks`를 기록하고 hook을 실행한다. 이 재사용은 후보 자체의 검증이나
검수를 대체하지 않는다.

새 `slice accept prepare`는 `yo.slice-accept-prepare-request/v1alpha3`을 사용한다.
ready gate와 사람이 작성한 message source만 필요하고 `push_remote`는 선택 사항이다.
이 명령은 작은 `yo.slice-close-prepare-request/v1alpha2`와 candidate, validation,
review evidence 수를 파생한다. compact 경로는 미검증 환경의 누락 command를 파생할
수 없으므로 알려진 미검증 환경이 없어야 한다. 그 매핑을 보존해야 하는 gate는 고정된
observed-metrics 경로를 사용한다. 따라서 사람이 실행 lane,
packet, 경과 시간 합계를 다시 만들어야 cleanup이 진행되는 병목이 없다. 고정된
이전 요청은 기존 observed-metrics 모양을 유지한다. close preparation은 표준
`close-metrics.json`을 발행하지만 cleanup을 직접 plan하거나 apply하지 않는다.

close plan은 요청한 파일에 plan을 직접 발행한 뒤 이미 수용된 결과를 소비한다.
로컬 worktree, 표준 임시 Slice contract, Slice branch를 제거하기 전에 정확한
ref, 검수 trailer, patch identity, worktree 청결 상태, binding, contract hash,
plan hash를 다시 검사한다. 전체 metrics 파일을 직접 작성하는 방식도 계속
지원한다. plan은 이
기록을 정확한 Slice candidate와 accepted commit에 결속하며 apply는 변경된
metrics를 거부한다. plan은 metrics를 포함해 보존할 직계 coordination 항목도
모두 나열하며, apply는 그 목록이 바뀌면 거절하고 해당 항목을 삭제하지 않는다.
plan은 제거할 worktree와 해당 Slice coordination 디렉터리 바깥에 저장한다.
통합 workflow는
[`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#review-and-integration)를
참고한다.

## 유용한 소유자

- hook 선택: [`hk.pkl`](https://github.com/Yon-Fandorin/yo/blob/develop/hk.pkl)
- 구조화된 저장소 검사: [`tools/xtask`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/xtask/src/lib.rs)
- Unix host compile 검사: [`tools/validation/yo-cli-unix-matrix.sh`](https://github.com/Yon-Fandorin/yo/blob/develop/tools/validation/yo-cli-unix-matrix.sh)
- rendering parity fixture: [`crates/yo-tui/tests/fixtures/rendering-parity/README.md`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-tui/tests/fixtures/rendering-parity/README.md)
- test 설명 정책: [`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md#test-code)
