# 계정 잔여량 조회

현재 Provider가 관리하는 계정 한도가 얼마나 남았는지 볼 때는 `yo account`를
사용한다. 저장된 Session이 관찰한 request·output·reasoning·cache token 수를 볼
때는 `yo usage SESSION_ID`를 사용한다. 두 보고서는 합계를 공유하지 않고 서로를
추론하지 않는다.

## 공개 명령

```bash
yo account
yo account kimi
yo account --detail
yo account codex --refresh
yo account grok --refresh
yo account kimi:default --refresh
yo account qwencloud:default --refresh
yo account kimi:default --refresh --format json
yo account codex:you@example.com --refresh
```

`SOURCE`를 생략하면 현재 지원되는 모든 account-capacity source를 보여준다. Provider만
(`kimi`) 지정하면 그 Provider에 저장된 모든 계정을 보여주고, `PROVIDER:ACCOUNT`를
지정하면 하나의 정확한 계정만 보여준다. `--refresh`가 없으면 로컬에 저장된 마지막
관측값만 표시하며 각 결과의 `Updated` 필드에 마지막 갱신 시각을 표시한다. 아직 한 번도
갱신하지 않은 계정은 `Not refreshed`와 `Never`로 표시된다. `--refresh`는 선택한 전체
범위에 적용되며, 성공한 관측값과 시각을 로컬 account-capacity cache에 저장한다.
여러 계정을 갱신할 때는 best-effort로 선택한 모든 source를 시도한다. 성공한 결과는
저장·표시하고 실패는 모아서 보고하며 종료 코드는 non-zero가 된다.
계정별 refresh failure는 선택한 결과 형식에만 포함한다. Text에서는 `Refresh failures` 아래에,
JSON에서는 기존 `errors` 배열에 표시하며, 이 예상된 실패를 위해 일반적인 stderr error를
중복해서 출력하지 않는다. 설정·직렬화·출력 대상 자체의 실패 같은 치명적 오류는 기존 stderr
error 경로를 사용한다.
성공한 관측 뒤 cache 저장에 실패한 경우에도 같은 구조화된 failure 결과에 포함하며, 방금
관측한 메모리상의 record는 계속 표시한다.

Codex app-server가 지원되는 protocol major 안의 검증되지 않은 minor version을
보고하면 refresh 자체는 성공하고 compatibility warning 하나를 낸다. Text에서는
계정 data 뒤의 `Refresh warnings`에 표시하고, JSON에서는 versioned stdout 문서를
바꾸지 않고 warning을 stderr로 보낸다. 표시하는 `userAgent`는 길이가 제한되고
터미널 안전 형태로 바뀐다. Major가 다르거나 version을 해석할 수 없으면 계속
refresh failure로 처리한다.

Text 출력은 선택한 범위가 계정 하나로 해석되면 기본으로 상세 화면을 사용하고, 여러
계정이면 테두리 없는 컬럼 표를 사용한다. 표가 interactive terminal 폭에 들어갈 때는
`PROVIDER`, `ACCOUNT`, `PLAN`, `LIMITS`, `UPDATED` 컬럼을 정렬한다. 폭이 부족하면
같은 결과를 상세 block으로 전환하며, pipe나 file 출력은 폭에 제한 없는 표 형식을 유지한다.
각 limit window의 정확한 잔여율 옆에 한 칸짜리
수직 level meter를 표시한다. `--detail`은 범위와 관계없이 상세 화면을 강제하며,
상세 limit 행도 같은 meter 계열을 수평 bar로 사용한다. Rich/ASCII glyph, meter 모양,
`{label}`/`{meter}`/`{percent}` 배치는 `yo-tui::meter`에서 재사용할 수 있고,
의미 기반 색상은 presentation 계층이 결정한다. 공통 출력 옵션인 `--ascii`는 account의
두 meter 형식 모두에 적용된다. `--format json`은 현재 `account`가 지원하며, 지원하지
않는 command에서는 실행 전에 미지원 오류로 거절한다. JSON은 terminal 폭, ANSI style,
glyph profile과 독립적이다. 갱신 또는 상세 명령 안내는 유용할 때만 표시하며 JSON
출력에는 사람용 명령 문구를 포함하지 않는다.

`codex`와 `grok`은 각각 로컬에 설치된 delegated host가 사용하는 계정을 뜻한다.
account-capacity를 실시간 갱신할 때는 두 host 모두 유효한 인증 이메일을 요구하고 이를 사람이 보는
계정 label로 표시한다. stable한 내부 account key는 별도로 보관한다. Codex는 native
account id가 있으면 이를 유지하고, Grok은 검증된 이메일을 identity evidence로
사용한다. 캐시된 결과는 이메일 label 또는 내부 key로 선택할 수 있다. 캐시가 없는
첫 실행에서는 갱신 전까지 `Local Codex` 또는 `Local Grok`과 `Account  Not resolved` 행으로
표시된다. 실제 로그인 계정을 확인하려면 `yo account PROVIDER --refresh`로 로컬 host에
질의한다. 이 미해결 행 자체는 선택 가능한 계정 이름이 아니며, 실제 `current` 계정명도 아니다.
`kimi:ACCOUNT`는 Yo에
`kimi-code-membership/v1` catalog profile 또는 정확한 canonical Kimi Code
complete binding과 정확한 Provider-and-Account credential로 이미 저장된 계정
하나를 지정한다. Binding fallback은 catalog-seed persistence 이전에 만든 연결을
유지하기 위한 것이며 custom endpoint를 허용하지 않는다. Kimi Platform API 계정은
다른 제품이므로 요청 전에 거절한다. `qwencloud:ACCOUNT`는 canonical Singapore
endpoint를 사용하는 정확히 저장된 Token Plan 연결을 받는다. 현재 QwenCloud browser
session으로 Personal Token Plan console을 읽으며, 저장된 `sk-sp-*` 모델 추론 키로는
이 console surface를 인증할 수 없다.

`--refresh`는 지정한 계정 소스 또는 소스들을 다시 관찰한다. 어느 경로도 Agent Session을 만들거나
모델 prompt를 보내거나 다른 Provider로 fallback하지 않는다. Codex는
로컬 app-server를 시작해 initialize한 뒤 `account/read`와 `account/rateLimits/read`를
각각 한 번 호출하고 종료한다. Grok은 `grok agent stdio`를 시작해 ACP v1로 initialize하고,
광고된
`cached_token` method로 한 번 인증한 뒤 정확한 `_meta.subscription_tier`와 필수 이메일
identity를 읽고 종료한다. 유효한 이메일이 없는 host 응답은 공용 default 계정으로
저장하지 않고 실패한다. 배포된 Grok ACP 서비스는 내부 billing
extension을 노출하지 않으므로 Yo는 Grok 공식 `unified.jsonl`의 마지막 1 MiB까지만
읽고, 주간 기간이 끝나지 않은 가장 최신의 완전한
`billing: fetched credits config` event만 사용한다. 그런 event가 없으면 사용량 창을
만들지 않고 인증된 plan만 보고한다. Kimi는 먼저 계정 등급명을 얻기 위해 인증한
`GET /coding/v1/me`를 한 번 수행한 뒤, 한도 조회를 위해 인증한
`GET /coding/v1/usages`를 한 번 수행한다. redirect와 retry는 비활성화하고 각 성공
body는 1 MiB로 제한한다. Provider plan 이름은 정확히 표시하며 Yo는 한도 크기로
이를 추론하지 않는다. QwenCloud는 저장된 account-session Cookie를 정확한 QwenCloud
console origin에만 사용한다. 그 Cookie 또는 bounded dashboard HTML response 하나에서
`sec_token`을 해석하고 현재 invocation 동안만 보유한 뒤, console의 `usage`,
`subscription`, `quota-config` request를 병렬로 보낸다. Redirect와 자동 HTTP retry는
비활성화하고 모든 request에는 8초 deadline, 모든 성공 body에는 1 MiB 한도를 둔다.
Provider가 5시간 window를 생략할 수 있으므로 그 window는 optional이며, 7일 window와
active `specCode`는 Provider가 작성한 관측값으로 유지한다.

### QwenCloud console session

Qwen 모델 요청에는 계속 저장된 `sk-sp-*` 추론 키와 그에 맞는 Token Plan
endpoint만 필요하다. 계정 잔여량 refresh는 대신 QwenCloud Billing을 열 수 있는
browser session이 필요하다. Yo는 다른 CLI를 설치하거나 Alibaba 관리 profile을
사용하지 않는다. 이 session은 같은 Provider-and-Account record의 모델 API key와
분리된 값으로 `credentials.yaml`에 저장한다.

1. `https://home.qwencloud.com`에 로그인하고 **Billing > Subscription**을 연다.
2. Browser Developer Tools의 **Network**를 연 뒤 새로고침하고,
   `cs-data.qwencloud.com`의 `api.json` request를 선택해 완전한 `Cookie` request
   header를 복사한다. 값에 `login_qwencloud_ticket`이 있어야 한다.
3. Interactive terminal에서 refresh를 실행한다.

   ```bash
   yo account qwencloud:default --refresh
   ```

   저장된 account session이 없으면 Yo가 terminal echo를 끄고 완전한 Cookie를 묻고
   로컬에 저장한다. 이후 refresh는 그 값을 재사용한다. QwenCloud가 session 만료를
   명시하면 Yo는 replacement를 한 번만 다시 묻고 저장한 뒤 정확히 한 번 갱신
   refresh를 수행한다. 입력이 필요한 비대화형 실행은 수행 가능한 안내와 함께
   실패한다.
4. `sec_token`은 별도 입력이나 저장 field가 필요 없다. Cookie 안에 있으면 거기서,
   없으면 dashboard response 한 번에서 유도하고 command가 끝나면 폐기한다.

Console 상태의 누락이나 만료는 이 계정 잔여량 refresh에만 영향을 준다. 저장된 Qwen
모델 API key를 교체하거나 비활성화하지 않고 모델 요청이나 다른 Provider fallback을
시작하지 않는다.

Cookie를 입력받기 전에 Yo는 공유 connection-operation lane을 획득하고 pending operation을
복구한 뒤 exact 저장 Token Plan binding과 그 API credential을 모두 확인하고 credential
revision을 capture한다. Account-session mutation은 no-echo prompt나 remote refresh 전에
그 관찰 revision에 묶어 준비한다. 따라서 credential이나 session이 동시에 바뀌면 조용히
다시 계획하거나 덮어쓰지 않고 conflict를 보고한다.

Text 출력은 사람용이다. `--format json`은 정확한 계정 하나일 때 versioned
`yo.account-capacity/v1alpha3` schema로, `yo account` 또는 Provider scope일 때는
계정이 하나여도 `accounts` 배열을 가진 `yo.account-capacity-list/v1alpha2` envelope로
출력한다. `account`는 사람이 읽는 label이고 stable key가 다를 때 `accountId`가 함께
나온다. 각 cache 결과에는 canonical `observedAt` timestamp도 포함된다. 일부 refresh가
실패하면 `errors` 배열을 추가하고 종료 코드는 non-zero가 된다. `--detail`은 text에만
적용하며 JSON shape는 고정한다. Provider의
percentage는 `0.01%`까지 보존하고 정수 값에는 불필요한 `.0`을 붙이지 않는다. count
값은 이 정밀도에서 used capacity를 올림해 보수적으로 정규화하므로 표시한 remaining
capacity가 정확한 비율보다 커지지 않는다. Provider가 보고한 exact `used`와 `limit`
count도 각 JSON window에 유지한다. JSON은 중립 snapshot으로 표현할 수 없는
Provider 고유 데이터를 allowlist한 optional `providerData`에도 보존한다. QwenCloud는
Provider가 보고한 exact percentage와 reset 값, active `specCode`, active-tier quota
값을 여기에 유지하지만 인증 material과 검증하지 않은 envelope field는 포함하지
않는다. 누락된 값은 absent 또는 Unknown으로 남고 Session token usage로 합성하지 않는다.

이전 `v1alpha1`과 `v1alpha2` shape는 historical contract로 유지한다. `v1alpha3`은
계정 label/key 분리와 refresh 오류 envelope를 fractional percentage, exact count,
allowlist한 `providerData` shape에 추가하며 consumer는 기록된 schema로 명시적으로
dispatch해야 한다.

## 참고한 upstream 코드

각 Provider adapter는 wire 동작의 근거가 된 정확한 upstream revision과 파일을
기록해야 한다. 이 링크는 구현 증거이지 Yo 계약의 두 번째 소유자가 아니며, 이후
upstream 변경이 근거를 조용히 바꾸지 못하도록 commit에 고정한다.

| 기능 | 고정한 upstream 소스 | Yo 적용 지점 |
|---|---|---|
| Codex 계정 잔여량 | OpenAI Codex commit `89650c66f2f3ff0d028d3f5d6d0b187b2ed49be5`: [app-server rate-limit request와 field](https://github.com/openai/codex/blob/89650c66f2f3ff0d028d3f5d6d0b187b2ed49be5/codex-rs/app-server/README.md#7-rate-limits-chatgpt), [v2 account protocol type](https://github.com/openai/codex/blob/89650c66f2f3ff0d028d3f5d6d0b187b2ed49be5/codex-rs/app-server-protocol/src/protocol/v2/account.rs) | [`delegated-codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-codex/src/lib.rs)가 app-server lifecycle을 소유하고 [`protocol.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-codex/src/protocol.rs)가 반환 bucket을 변환한다. |
| Grok 계정 plan과 최신 billing 관찰 | xAI Grok Build commit `9684fa3cdbf2995e30ea8b9b637f1db008f144fc`: [ACP authenticate response 구성](https://github.com/xai-org/grok-build/blob/9684fa3cdbf2995e30ea8b9b637f1db008f144fc/crates/codegen/xai-grok-shell/src/agent/mvp_agent/mod.rs), [typed authentication metadata](https://github.com/xai-org/grok-build/blob/9684fa3cdbf2995e30ea8b9b637f1db008f144fc/crates/codegen/xai-grok-shell/src/auth/meta.rs), [bounded unified billing log event](https://github.com/xai-org/grok-build/blob/9684fa3cdbf2995e30ea8b9b637f1db008f144fc/crates/codegen/xai-grok-shell/src/extensions/billing.rs). 설치된 Grok CLI `1.0.5 (5115b46bc9)`에서도 정확한 경계를 관찰했다. | [`delegated-grok`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-grok/src/lib.rs)가 initialize-authenticate-shutdown read를 소유하고, [`billing_log.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-grok/src/billing_log.rs)가 bounded tail에서 공식 current-period event만 변환한다. |
| Kimi Code 계정 잔여량 | MoonshotAI Kimi Code commit `21f7ef64f0851504227617f4501bf8359031d9a5`: canonical `/me` request와 `user_level_name`의 근거인 [`managed-userinfo.ts`](https://github.com/MoonshotAI/kimi-code/blob/21f7ef64f0851504227617f4501bf8359031d9a5/packages/oauth/src/managed-userinfo.ts), `/usages`, weekly summary, rolling window, fixed-point booster balance의 근거인 [`managed-usage.ts`](https://github.com/MoonshotAI/kimi-code/blob/21f7ef64f0851504227617f4501bf8359031d9a5/packages/oauth/src/managed-usage.ts) | [`usage.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/kimi_catalog/usage.rs)가 Kimi catalog seed 옆에서 제품 확인, 두 exact request, bounded parser, 중립 snapshot 변환을 소유한다. |
| QwenCloud Personal Token Plan 잔여량 | OmniRoute commit `825f8feea73daead73cf6832bed7c61531f9c065`: [`qwenTokenPlanQuotaFetcher.ts`](https://github.com/diegosouzapw/OmniRoute/blob/825f8feea73daead73cf6832bed7c61531f9c065/open-sse/services/qwenTokenPlanQuotaFetcher.ts)는 관찰한 QwenCloud console gateway, cookie/`sec_token` 분리, personal-plan method 세 개, optional 5시간 window를 기록하고, [request와 parser fixture](https://github.com/diegosouzapw/OmniRoute/blob/825f8feea73daead73cf6832bed7c61531f9c065/tests/unit/qwen-token-plan-quota-fetcher.test.ts)는 weekly-only, dual-window, expired-session, token-resolution 사례를 구분한다. | [`qwencloud.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/command/account/qwencloud.rs)는 session을 고정된 QwenCloud origin에만 사용하고 bounded no-retry read를 수행해 Provider가 작성한 plan과 window를 중립 snapshot으로 변환한다. |

Adapter를 바꿀 때는 새 upstream commit을 확인해 source link를 다시 고정하고,
차이를 판별하는 fixture와 정확한 live boundary를 검증한다. 고정하지 않은 `main`
branch를 인용하거나 UI 출력만 보고 private endpoint를 추론하지 않는다.

## 실패 경계

- 저장된 Kimi 계정이나 credential이 없으면 local configuration error이며 요청을
  보내지 않는다.
- Grok cached login이나 유효한 email이 없거나 subscription tier가 없거나 문자열이 아니거나
  안전하지 않으면 refresh를 실패시킨다. Grok billing log가 없거나 사용할 수 없으면 usage
  window만 생략한다. Direct xAI 접근으로 fallback하거나 Grok credential file을
  읽거나 identity metadata를 노출하지 않는다.
- 저장된 QwenCloud account session이 없으면 no-echo interactive capture를 한 번
  시작한다. 명시적으로 만료된 session이면 replacement capture 한 번과 최대 한 번의
  갱신 refresh만 수행한다. 저장된 inference key를 console 인증에 대신 쓰거나 browser
  session을 설정 가능한 origin으로 전송하지 않는다.
- QwenCloud는 이 Personal Token Plan console gateway를 안정적인 공개 API로 제공하지
  않는다. Upstream console이 바뀌면 고정 source와 fixture 갱신이 필요할 수 있으며,
  Yo는 대체 shape를 추측하지 않고 실패한다.
- 성공이 아닌 status, redirect, 잘못된 media type, malformed JSON, 누락되거나
  안전하지 않은 Kimi 등급명, 잘못된 reset time, 0인 limit, 초과 row, 초과 byte는
  부분적인 정상 보고서 대신 refresh 전체를 실패시킨다.
- Secret은 정확한 인증 request에만 쓰며 error, snapshot, text, JSON, test
  evidence에 남기지 않는다.
- Account-capacity 실패는 Session을 시작하거나 Provider를 재시도하거나 저장된
  connection state를 바꾸지 않는다.
