# 계정 잔여량 조회

현재 Provider가 관리하는 계정 한도가 얼마나 남았는지 볼 때는 `yo account`를
사용한다. 저장된 Session이 관찰한 request·output·reasoning·cache token 수를 볼
때는 `yo usage SESSION_ID`를 사용한다. 두 보고서는 합계를 공유하지 않고 서로를
추론하지 않는다.

## 공개 명령

```bash
yo account codex --refresh
yo account grok --refresh
yo account kimi:default --refresh
yo account qwencloud:default --refresh
yo account kimi:default --refresh --format json
```

`codex`와 `grok`은 각각 로컬에 설치된 delegated host가 사용하는 계정을 뜻한다.
`kimi:ACCOUNT`는 Yo에
`kimi-code-membership/v1` catalog profile 또는 정확한 canonical Kimi Code
complete binding과 정확한 Provider-and-Account credential로 이미 저장된 계정
하나를 지정한다. Binding fallback은 catalog-seed persistence 이전에 만든 연결을
유지하기 위한 것이며 custom endpoint를 허용하지 않는다. Kimi Platform API 계정은
다른 제품이므로 요청 전에 거절한다. `qwencloud:ACCOUNT`는 canonical Singapore
endpoint를 사용하는 정확히 저장된 Token Plan 연결을 받는다. 현재 QwenCloud browser
session으로 Personal Token Plan console을 읽으며, 저장된 `sk-sp-*` 모델 추론 키로는
이 console surface를 인증할 수 없다.

`--refresh`는 지정한 계정 소스를 다시 관찰한다. 어느 경로도 Agent Session을 만들거나
모델 prompt를 보내거나 다른 Provider로 fallback하지 않는다. Codex는
로컬 app-server를 시작해 initialize한 뒤 `account/rateLimits/read`를 한 번 호출하고
종료한다. Grok은 `grok agent stdio`를 시작해 ACP v1로 initialize하고, 광고된
`cached_token` method로 한 번 인증한 뒤 정확한 `_meta.subscription_tier`를 읽고
종료한다. Identity metadata는 무시한다. 배포된 Grok ACP 서비스는 내부 billing
extension을 노출하지 않으므로 Yo는 Grok 공식 `unified.jsonl`의 마지막 1 MiB까지만
읽고, 주간 기간이 끝나지 않은 가장 최신의 완전한
`billing: fetched credits config` event만 사용한다. 그런 event가 없으면 사용량 창을
만들지 않고 인증된 plan만 보고한다. Kimi는 먼저 계정 등급명을 얻기 위해 인증한
`GET /coding/v1/me`를 한 번 수행한 뒤, 한도 조회를 위해 인증한
`GET /coding/v1/usages`를 한 번 수행한다. redirect와 retry는 비활성화하고 각 성공
body는 1 MiB로 제한한다. Provider plan 이름은 정확히 표시하며 Yo는 한도 크기로
이를 추론하지 않는다. QwenCloud는 완전한 `QWEN_CLOUD_COOKIE`를 정확한 QwenCloud
console origin에만 사용한다. Cookie 또는 bounded dashboard HTML response 하나에서
`sec_token`을 해석한 뒤 console의 `usage`, `subscription`, `quota-config` request를
병렬로 보낸다. Redirect와 retry는 비활성화하고 모든 request에는 8초 deadline,
모든 성공 body에는 1 MiB 한도를 둔다. Provider가 5시간 window를 생략할 수 있으므로
그 window는 optional이며, 7일 window와 active `specCode`는 Provider가 작성한
관측값으로 유지한다.

### QwenCloud console session

Qwen 모델 요청에는 계속 저장된 `sk-sp-*` 추론 키와 그에 맞는 Token Plan
endpoint만 필요하다. 계정 잔여량 refresh는 대신 QwenCloud Billing을 열 수 있는
browser session이 필요하다. Yo는 다른 CLI를 설치하거나 Alibaba 관리 profile을
사용하거나 browser cookie를 영속화하지 않는다.

1. `https://home.qwencloud.com`에 로그인하고 **Billing > Subscription**을 연다.
2. Browser Developer Tools의 **Network**를 연 뒤 새로고침하고,
   `cs-data.qwencloud.com`의 `api.json` request를 선택해 완전한 `Cookie` request
   header를 복사한다. 값에 `login_qwencloud_ticket`이 있어야 한다.
3. shell의 숨김 입력으로 붙여 넣는다.

   ```bash
   read -rs QWEN_CLOUD_COOKIE
   export QWEN_CLOUD_COOKIE
   ```

   붙여 넣은 뒤 Enter를 누른다. Cookie는 process-local로만 남고 Yo 설정에는 쓰지
   않는다.
4. Yo는 보통 로그인된 dashboard에서 `sec_token`을 해석한다. 이 과정만 실패하면
   request의 `sec_token` form field를 복사해 같은 방식으로 제공한다.

   ```bash
   read -rs QWEN_CLOUD_SEC_TOKEN
   export QWEN_CLOUD_SEC_TOKEN
   ```
5. 한 번 refresh한 뒤 두 값을 shell에서 모두 지운다.

   ```bash
   yo account qwencloud:default --refresh
   unset QWEN_CLOUD_COOKIE QWEN_CLOUD_SEC_TOKEN
   ```

Console 상태가 없거나 만료되면 이 계정 잔여량 refresh만 실패한다. 저장된 Qwen 모델 연결을
비활성화하거나 모델 요청을 보내지 않는다.

Text 출력은 사람용이다. `--format json`은 같은 Provider 중립 snapshot을 agent가
읽을 수 있는 versioned `yo.account-capacity/v1alpha1` schema로 출력한다. Provider의
count 값은 보수적으로 정규화한다. used percentage를 올림하므로 표시한 remaining
percentage가 정확한 잔여 비율보다 커지지 않는다. 누락된 값은 absent 또는 Unknown으로
남고 Session token usage로 합성하지 않는다.

## 참고한 upstream 코드

각 Provider adapter는 wire 동작의 근거가 된 정확한 upstream revision과 파일을
기록해야 한다. 이 링크는 구현 증거이지 Yo 계약의 두 번째 소유자가 아니며, 이후
upstream 변경이 근거를 조용히 바꾸지 못하도록 commit에 고정한다.

| 기능 | 고정한 upstream 소스 | Yo 적용 지점 |
|---|---|---|
| Codex 계정 잔여량 | OpenAI Codex commit `89650c66f2f3ff0d028d3f5d6d0b187b2ed49be5`: [app-server rate-limit request와 field](https://github.com/openai/codex/blob/89650c66f2f3ff0d028d3f5d6d0b187b2ed49be5/codex-rs/app-server/README.md#7-rate-limits-chatgpt), [v2 account protocol type](https://github.com/openai/codex/blob/89650c66f2f3ff0d028d3f5d6d0b187b2ed49be5/codex-rs/app-server-protocol/src/protocol/v2/account.rs) | [`delegated-codex`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-codex/src/lib.rs)가 app-server lifecycle을 소유하고 [`protocol.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-codex/src/protocol.rs)가 반환 bucket을 변환한다. |
| Grok 계정 plan과 최신 billing 관찰 | xAI Grok Build commit `9684fa3cdbf2995e30ea8b9b637f1db008f144fc`: [ACP authenticate response 구성](https://github.com/xai-org/grok-build/blob/9684fa3cdbf2995e30ea8b9b637f1db008f144fc/crates/codegen/xai-grok-shell/src/agent/mvp_agent/mod.rs), [typed authentication metadata](https://github.com/xai-org/grok-build/blob/9684fa3cdbf2995e30ea8b9b637f1db008f144fc/crates/codegen/xai-grok-shell/src/auth/meta.rs), [bounded unified billing log event](https://github.com/xai-org/grok-build/blob/9684fa3cdbf2995e30ea8b9b637f1db008f144fc/crates/codegen/xai-grok-shell/src/extensions/billing.rs). 설치된 Grok CLI `1.0.5 (5115b46bc9)`에서도 정확한 경계를 관찰했다. | [`delegated-grok`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-grok/src/lib.rs)가 initialize-authenticate-shutdown read를 소유하고, [`billing_log.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/delegated-grok/src/billing_log.rs)가 bounded tail에서 공식 current-period event만 변환한다. |
| Kimi Code 계정 잔여량 | MoonshotAI Kimi Code commit `21f7ef64f0851504227617f4501bf8359031d9a5`: canonical `/me` request와 `user_level_name`의 근거인 [`managed-userinfo.ts`](https://github.com/MoonshotAI/kimi-code/blob/21f7ef64f0851504227617f4501bf8359031d9a5/packages/oauth/src/managed-userinfo.ts), `/usages`, weekly summary, rolling window, fixed-point booster balance의 근거인 [`managed-usage.ts`](https://github.com/MoonshotAI/kimi-code/blob/21f7ef64f0851504227617f4501bf8359031d9a5/packages/oauth/src/managed-usage.ts) | [`usage.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/kimi_catalog/usage.rs)가 Kimi catalog seed 옆에서 제품 확인, 두 exact request, bounded parser, 중립 snapshot 변환을 소유한다. |
| QwenCloud Personal Token Plan 잔여량 | OmniRoute commit `825f8feea73daead73cf6832bed7c61531f9c065`: [`qwenTokenPlanQuotaFetcher.ts`](https://github.com/diegosouzapw/OmniRoute/blob/825f8feea73daead73cf6832bed7c61531f9c065/open-sse/services/qwenTokenPlanQuotaFetcher.ts)는 관찰한 QwenCloud console gateway, cookie/`sec_token` 분리, personal-plan method 세 개, optional 5시간 window를 기록하고, [request와 parser fixture](https://github.com/diegosouzapw/OmniRoute/blob/825f8feea73daead73cf6832bed7c61531f9c065/tests/unit/qwen-token-plan-quota-fetcher.test.ts)는 weekly-only, dual-window, expired-session, token-resolution 사례를 구분한다. | [`qwencloud.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/account/qwencloud.rs)는 session을 고정된 QwenCloud origin에만 사용하고 bounded no-retry read를 수행해 Provider가 작성한 plan과 window를 중립 snapshot으로 변환한다. |

Adapter를 바꿀 때는 새 upstream commit을 확인해 source link를 다시 고정하고,
차이를 판별하는 fixture와 정확한 live boundary를 검증한다. 고정하지 않은 `main`
branch를 인용하거나 UI 출력만 보고 private endpoint를 추론하지 않는다.

## 실패 경계

- 저장된 Kimi 계정이나 credential이 없으면 local configuration error이며 요청을
  보내지 않는다.
- Grok cached login이 없거나 subscription tier가 없거나 문자열이 아니거나 안전하지
  않으면 refresh를 실패시킨다. Grok billing log가 없거나 사용할 수 없으면 usage
  window만 생략한다. Direct xAI 접근으로 fallback하거나 Grok credential file을
  읽거나 identity metadata를 노출하지 않는다.
- Console cookie가 없거나 QwenCloud cookie가 아니거나 만료되면 QwenCloud refresh는
  모델 요청 전에 실패한다. Yo는 저장된 inference key를 console 인증에 대신 쓰거나,
  browser session을 영속화하거나, 설정 가능한 origin으로 전송하지 않는다.
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
