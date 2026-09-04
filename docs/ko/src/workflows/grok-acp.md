# Grok ACP upstream 따라가기

설치된 Grok CLI의 ACP handshake, 인증 method, Session 생명주기, update 또는
permission request가 바뀌었을 때 이 흐름을 사용한다. 이는 운영 검증
가이드이며 backend 의미의 두 번째 소유자가 아니다.

## 범위와 소유권

`host:grok`은 delegated agent host다. Model Provider가 아니며 Yo의 external-model
credential repository를 사용하지 않는다. 독립
[`yo-backend-delegated-grok`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-grok/src/lib.rs)
crate가 `grok agent stdio`, ACP v1 JSON-RPC, 인증, Session correlation, event
변환, permission, 취소, process cleanup을 소유한다.
[`yo-backend`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/foundation/src/lib.rs)는
generic `BackendAdapter` lifecycle과 bounded process 메커니즘을 소유한다.
[`yo-core`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/contract.rs)는
그 lifecycle을 provider 중립 `AgentBackend` 계약으로 특수화하고 semantic
runtime을 소유한다.

현재 adapter는 설치된 CLI가 광고한 `cached_token` 인증 method만 사용한다.
운영자는 `grok login`으로 인증하며 Yo는 그 token을 읽거나 복사하거나
갱신하거나 저장하지 않는다. 직접 xAI API 또는 OAuth 통합은 별도의 Provider
설계이며 이 host adapter 안의 fallback이 되어서는 안 된다.

`yo account grok --refresh`는 initialize-authenticate prefix만 재사용한 뒤 child를
종료한다. 성공한 authentication metadata의 정확한 `subscription_tier`를 Provider
중립 account snapshot으로 변환하고, 필수 검증 이메일을 사람이 보는 계정 label과
stable identity evidence로 함께 사용한다. team identity와 authentication mode
metadata는 무시한다. Agent Session을 만들거나 prompt를 보내지 않는다. 설치된
Grok CLI 1.0.5는 더 새로운 `x.ai/billing` extension을 노출하지 않으므로 plan
이름에서 quota window를 추론하지 않는다.

## Compatibility 계약

adapter가 사용하는 wire surface는 fail-closed로 유지한다.

- 빈 client capability와 ACP protocol version 1로 initialize한다.
- 설치된 agent가 `cached_token`을 광고해야 하며 정확히 그 method로 인증한다.
- 유효한 인증 이메일을 요구하고 계정 label과 stable identity evidence로 사용한다.
- account snapshot을 만들 때는 크기가 제한되고 비어 있지 않으며 앞뒤 공백과
  제어문자가 없는 정확한 subscription tier만 허용한다.
- `session/new`로 만들고 agent가 load 지원을 광고할 때만 `session/load`로
  재개한다.
- 모든 response, Session update, permission request, terminal prompt 결과를
  활성 request와 Session에 연결한다.
- text, thought, tool, permission, 취소, stop-reason message를 provider 중립
  backend event로 변환한다.
- message, queue, request wait, 보존 stderr, process shutdown을 제한한다.

실행 파일 version만으로 compatibility를 추정하지 않는다. 후보의 ACP 동작을
확인하고 허용한 형태를 malformed, mismatched, unsupported message와 구분하는
결정론적 fixture를 유지한다.

외부 검토에서는 admission v1alpha5가 먼저 stdin이 이미 EOF인 상태로 동결된
native reviewer profile을 시작한다.

```text
grok --sandbox read-only --permission-mode dontAsk --tools Read,Grep \
  --no-subagents --disable-web-search agent stdio
```

이 bounded startup probe는 ContextBuild나 packet 발행보다 먼저 실행된다. prompt,
ACP initialize, Session request 또는 비공개 packet을 전송하지 않는다. native profile이
Linux에서 시작되지 않으면 Yo는 자신이 소유한 외곽 profile
`yo-bwrap-read-only/v1alpha1`을 정확히 한 번 시도할 수 있다. 이는 불변 review packet을
위한 제한된 compatibility 경계이지 범용 host sandbox가 아니다.

외곽 profile은 bubblewrap으로 root와 review workspace를 read-only로 만들고 user, PID,
IPC, UTS namespace를 분리하며 capability를 모두 제거한다. 임시·runtime directory는
비공개로 만들고 알려진 Docker, libvirt, LXD runtime endpoint를 가린다. 정확한 Grok
state directory와 Session repository만 쓸 수 있으며 Provider network access는 유지한다.
이 경계 안의 Grok은 native sandbox를 끄고 `dontAsk` permission, 빈 tool 집합, web search
및 subagent 비활성화 상태로 실행된다. delegated backend는 정확한 profile marker,
read-only sentinel mount, 차단된 workspace 쓰기를 독립적으로 검증한 뒤에만 이를
허용한다.

admission은 delivery가 `grok-native-read-only/v1alpha1` 또는
`yo-bwrap-read-only/v1alpha1` 중 무엇을 사용했는지 기록하며 delivery claim과 receipt가
그 선택을 보존한다. finding-resolution continuation은 새 claim을 게시하기 전에 직전
delivery receipt에 기록된 것과 정확히 같은 isolation을 선택해야 한다. 물리적 isolation이
기록되지 않은 legacy receipt는 isolation을 선택하는 continuation을 승인할 수 없다. 실행
파일 누락, 미지원 platform, 불완전한 경계, attestation 실패,
native와 outer startup 모두의 실패는 host를 unavailable로 만들며 격리 없는 검토로
약화하지 않는다. Grok 1.0.13은 제한된 Linux container에서 native bubblewrap profile이
container-runtime socket을 가리지 못할 때 외곽 profile이 필요할 수 있다. 설치 version
문자열이나 쓰기 가능한 `~/.grok`만으로 review readiness를 증명하지 않는다.

outer isolation claim을 게시하기 전에 delivery는 `yo`를 build하는 데 쓰는 trusted
current-develop source가 delegated runner capability manifest에서 그 exact isolation을
광고하는지도 요구한다. 새 review isolation을 도입하는 Slice는 검토되지 않은 자신의
후보를 자기 packet을 반출하는 runner로 사용할 수 없다. 이미 활성화된 별도 route로
검토하고 통합한 뒤 새 isolation을 dogfood한다. capability 증거가 없으면 claim이나 host
request 전에 중단한다.

## 집중 검증

먼저 결정론적 adapter test를 실행한다.

```bash
cargo test -p yo-backend-delegated-grok
```

CLI가 설치되어 있고 `grok login`이 완료된 환경에서는 inference Turn을
소비하지 않고 실제 initialize, cached-token 인증, cleanup 경계를 확인한다.

```bash
cargo test -p yo-backend-delegated-grok \
  local_grok_authenticates_and_shuts_down_without_a_session \
  -- --ignored --nocapture

yo account grok --refresh
```

실제 prompt나 TUI smoke run은 외부 service capacity를 소비한다. Turn 수준
compatibility를 검증해야 할 때만 한 번 실행하고 설치 version, 인증 상태,
정확한 command, 관찰한 route, 미검증 환경을 기록한다. 마지막으로
[Slice 종료 기준선](../validation/#slice-종료-기준선)을 실행하고 변경된
compatibility 경계에 fresh-context review를 받는다.
