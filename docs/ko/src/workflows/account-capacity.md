# 계정 잔여량 조회

현재 Provider가 관리하는 계정 한도가 얼마나 남았는지 볼 때는 `yo account`를
사용한다. 저장된 Session이 관찰한 request·output·reasoning·cache token 수를 볼
때는 `yo usage SESSION_ID`를 사용한다. 두 보고서는 합계를 공유하지 않고 서로를
추론하지 않는다.

## 공개 명령

```bash
yo account codex --refresh
yo account kimi:default --refresh
yo account kimi:default --refresh --format json
```

`codex`는 로컬에 설치된 Codex 계정을 뜻한다. `kimi:ACCOUNT`는 Yo에
`kimi-code-membership/v1` catalog profile 또는 정확한 canonical Kimi Code
complete binding과 정확한 Provider-and-Account credential로 이미 저장된 계정
하나를 지정한다. Binding fallback은 catalog-seed persistence 이전에 만든 연결을
유지하기 위한 것이며 custom endpoint를 허용하지 않는다. Kimi Platform API 계정은
다른 제품이므로 요청 전에 거절한다.

`--refresh`는 의도적으로 live read를 수행한다. 어느 경로도 Agent Session을
만들거나 모델 prompt를 보내거나 다른 Provider로 fallback하지 않는다. Codex는
로컬 app-server를 시작해 initialize한 뒤 `account/rateLimits/read`를 한 번 호출하고
종료한다. Kimi는 먼저 계정 등급명을 얻기 위해 인증한 `GET /coding/v1/me`를 한 번
수행한 뒤, 한도 조회를 위해 인증한 `GET /coding/v1/usages`를 한 번 수행한다.
redirect와 retry는 비활성화하고 각 성공 body는 1 MiB로 제한한다. Kimi 계정
등급명은 Provider가 보고한 정확한 값을 plan으로 표시하며, Yo는 한도 크기로 이를
추론하지 않는다.

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
| Kimi Code 계정 잔여량 | MoonshotAI Kimi Code commit `21f7ef64f0851504227617f4501bf8359031d9a5`: canonical `/me` request와 `user_level_name`의 근거인 [`managed-userinfo.ts`](https://github.com/MoonshotAI/kimi-code/blob/21f7ef64f0851504227617f4501bf8359031d9a5/packages/oauth/src/managed-userinfo.ts), `/usages`, weekly summary, rolling window, fixed-point booster balance의 근거인 [`managed-usage.ts`](https://github.com/MoonshotAI/kimi-code/blob/21f7ef64f0851504227617f4501bf8359031d9a5/packages/oauth/src/managed-usage.ts) | [`usage.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/kimi_catalog/usage.rs)가 Kimi catalog seed 옆에서 제품 확인, 두 exact request, bounded parser, 중립 snapshot 변환을 소유한다. |

Adapter를 바꿀 때는 새 upstream commit을 확인해 source link를 다시 고정하고,
차이를 판별하는 fixture와 정확한 live boundary를 검증한다. 고정하지 않은 `main`
branch를 인용하거나 UI 출력만 보고 private endpoint를 추론하지 않는다.

## 실패 경계

- 저장된 Kimi 계정이나 credential이 없으면 local configuration error이며 요청을
  보내지 않는다.
- 성공이 아닌 status, redirect, 잘못된 media type, malformed JSON, 누락되거나
  안전하지 않은 Kimi 등급명, 잘못된 reset time, 0인 limit, 초과 row, 초과 byte는
  부분적인 정상 보고서 대신 refresh 전체를 실패시킨다.
- Secret은 정확한 인증 request에만 쓰며 error, snapshot, text, JSON, test
  evidence에 남기지 않는다.
- Account-capacity 실패는 Session을 시작하거나 Provider를 재시도하거나 저장된
  connection state를 바꾸지 않는다.
