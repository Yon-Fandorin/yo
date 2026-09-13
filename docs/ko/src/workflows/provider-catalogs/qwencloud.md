# QwenCloud 카탈로그 관리

QwenCloud는 release-known static-registry 사례다. Alibaba Cloud가 정확한 plan
allowlist와 plan별 endpoint를 공개한다. 등록은 구조적 admission만 수행하며, 일반 모델 사용이
나중에 제공된 credential로 선택한 행을 실제 사용할 수 있는지 확인한다.

## 공식 출처

해당 profile에 맞는 Alibaba Cloud 공식 페이지를 사용한다.

- [Coding Plan](https://www.alibabacloud.com/help/en/model-studio/coding-plan)에서
  exact model allowlist와 Coding Plan endpoint를 확인한다.
- [Token Plan (Team Edition)](https://www.alibabacloud.com/help/en/model-studio/token-plan-overview)에서
  exact model·capability 표를 확인한다.
- Endpoint, region, protocol, key type을 확인해야 하면 해당 quick-start
  페이지를 함께 사용한다.

가까운 이름을 보고 model version을 추론하지 않는다. 공식 plan 목록에 있다는
사실을 특정 계정에 활성 seat, quota, entitlement가 있다는 증거로 사용하지
않는다.

## 코드 소유 경계

Static profile definition, endpoint, row, typed capability, deterministic
ordering은
[`catalog.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/providers/qwencloud/src/catalog.rs)가
소유한다. Configuration은
[`config.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/state/config.rs)에서
profile을 non-routable seed로 해석한다. Shared selection과 recoverable connection
transaction은
[`external.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/command/connect/external.rs)와
[`picker.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/command/connect/picker.rs)에
있다.

## 갱신 절차

1. 기존 catalog profile 하나를 선택하고 정확한 공식 allowlist와 endpoint를
   확인한다. Plan이나 region의 의미가 그 profile과 더 이상 맞지 않으면 예전
   profile을 조용히 재해석하지 말고 SOT-first 작업으로 새 versioned profile을
   정의한다.
2. ModelId, modality, tool support, reasoning presentation, context limit,
   output limit, endpoint, dialect의 field-level old/new 표를 만든다. 공식 근거가
   없으면 다른 vendor 페이지나 model-family 추정값으로 채우지 말고 미확인으로
   표시한다.
3. 해당 `CatalogDefinition`과 `CatalogRow` data 또는 공식 사실을 올바르게
   표현하는 가장 작은 helper만 수정한다. yo에 필요한 runtime interface가
   없으면 유효한 image-only 행이나 다른 미지원 행을 표시하되 disabled 상태로
   유지한다.
4. 바뀐 profile의 exact row/order assertion을 추가한다. Duplicate·unknown
   profile, secret 입력 전 disabled-row 거부, secret·mutation 전 picker 취소,
   exact three-part selection을 테스트한다.
5. Stale-managed-row 회귀를 유지한다. 현재 registry 밖의 이전 저장 행은
   startup/recovery에서 계속 사용할 수 있지만 새 catalog candidate가 될 수는
   없다.

집중 검사:

```bash
cargo test --locked -p yo-provider-qwencloud catalog
cargo test --locked -p yo-cli qwencloud_catalog
cargo test --locked -p yo-cli command::connect::picker
```

이 갱신 경로는 QwenCloud plan을 열거하는 network request를 의도적으로 수행하지
않는다. 공식 authenticated account inventory를 authority로 사용하려면 static
table을 임시로 확장하지 말고 새로운 discovery 설계로 취급한다.

## Explicit general API image connection

승인된 일반 API 이미지 envelope는 plan 카탈로그와 별개다. 국제 Chat endpoint의
정확한 `qwen3.8-flash`를 선택한다. 기존 텍스트 binding은 명시적인 complete-definition
import 전까지 동작을 유지하며 catalog capability flag로 이미지 입력을 켜지 않는다.
닫힌 envelope는 승인된
[service binding](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.model.service-binding.md)과
[Chat request contract](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.connector.openai-chat-completions.md),
현재 공식
[Flash guide](https://docs.qwencloud.com/developer-guides/getting-started/latest-model)를 참고한다.

다음 공개 definition을 절대 경로에 저장하고
`yo connect --from /absolute/qwen-image.yaml`을 실행한다. 확인 전에 endpoint·model·
advisory 이미지 계산·비활성 thinking·semantic replay·일반 API 종량제를 검토하고,
숨김 입력창에 일반 API key를 입력한다.

```yaml
provider: qwencloud
provider_display_name: QwenCloud
account: general
account_display_name: General API
base_url: https://dashscope-intl.aliyuncs.com/compatible-mode/v1
profile:
  api_dialect: openai-chat-completions
  tokenizer_profile: utf8-bytes/v1
  input_token_limit: 991808
  max_output_tokens: 131072
  reasoning_parameters: {}
  optional_request_parameters:
    enable_thinking: false
    preserve_thinking: false
  tool_capability_policy: local-tools/v1
  replay_profile: semantic-only/v1
  image_input_profile: qwencloud-general-png-advisory/v1
models:
  - model: qwen3.8-flash
```

Yo는 선택된 `config.yaml` 옆의 `credentials.yaml`에 Provider `qwencloud`, Account
`general`로 key를 저장하고 공개 binding은 `connections.yaml`에 저장한다. Linux의
기본 디렉터리는 `~/.config/yo`다. 이 파일은 소유자 전용 권한을 가진 로컬 평문 YAML이며
암호화된 vault가 아니다. 일반 시작·복구는 저장된 key를 재사용한다. 검증은 key를 보존하고
임시 probe 파일에 복사하거나 다시 입력해 달라고 요청하지 않는다. 별도
`qwencloud:default` plan 계정은 분리해 유지한다.

`yo --model qwencloud:general:qwen3.8-flash`로 시작해 Ctrl+V 또는 기존 파일 이미지
흐름으로 PNG를 첨부하고 제출한다. Qwen은 활성화된 function tools와 명시적
`tool_choice: auto`를 보내며 idle `/compact`는 tools를 생략한다. 압축과 새 프로세스의
`yo --continue` 뒤에도 request option은 비활성 상태다. 전체 흐름과 무료 서비스 근거는
[terminal checks](../../validation/terminal-matrix.md#qwencloud-general-image-journey)를 참고한다.

일반 API는 종량제다. 무료 실제 서비스 검증은 전송 전에 이 정확한 model의 현재 무료
쿼터와 [`Free quota only`](https://docs.qwencloud.com/resources/free-quota) 보호를
별도로 확인한다. Yo는 쿼터를 조회하거나 Provider 설정을 켜지 않으며 advisory 이미지
계산은 무료 entitlement를 입증하지 않는다.

집중 구현 검사:

```bash
cargo test --locked -p yo-core model_service
cargo test --locked -p yo-connector-openai-chat-completions
cargo test --locked -p yo-backend-managed
cargo test --locked -p yo-cli
```
