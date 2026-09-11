# OpenRouter 카탈로그 관리

OpenRouter는 runtime-discovery 사례다. yo는 connection 시점에 설정된 계정을
조회하고, 인증된 응답을 normalize한 뒤, 지원·미지원 행을 shared picker로
표시한다. 갱신을 쉽게 하려는 이유만으로 이를 release에 고정된 모델 목록으로
바꾸지 않는다.

## 공식 출처

OpenRouter의 공식
[Models API](https://openrouter.ai/docs/api/api-reference/models/list-all-models-and-their-properties)를
사용한다. 이 문서는 인증된 `GET /api/v1/models` 응답과 model metadata를
설명한다. 공식 출처라도 live response는 untrusted input으로 취급한다.

## 코드 소유 경계

| 책임 | 소유자 |
|---|---|
| 크기가 제한된 authenticated transport | [`discovery/transport.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/providers/openrouter/src/discovery/transport.rs) |
| Response parsing, normalization, availability, authored override | [`discovery.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/providers/openrouter/src/discovery.rs)와 [`discovery/normalize.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/providers/openrouter/src/discovery/normalize.rs) |
| 설정된 discovery seed | [`config.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/state/config.rs) |
| Connect orchestration과 picker handoff | [`external.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/command/connect/external.rs)와 [`picker.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/command/connect/picker.rs) |

## 갱신 절차

1. 현재 공식 schema와 yo가 실제 사용하는 필드만 비교한다. response field가
   추가됐다는 사실만으로 yo capability가 되지는 않는다. 사용하는 필드의
   이름이나 의미가 바뀌면 contract와 compatibility를 함께 감사한다.
2. Transport bound나 normalization을 바꿀 때는 이전·새 형태를 구별하는
   fixture를 추가한다. Same-origin redirect policy, secret-safe diagnostic,
   response bound, typed disabled reason을 유지한다.
3. Authored-field provenance를 따로 확인한다. Remote context/output limit은 그
   필드가 직접 작성되지 않았을 때 적용한다. 관련 없는 authored model field가
   remote limit 적용을 막으면 안 된다.
4. Shadow list나 count가 아니라 authoritative picker handoff를 테스트한다.
   표시되는 Provider, Account, disabled reason, 선택된 exact ModelId는 connect가
   소비하는 normalized row에서 와야 한다.
5. Catalog 갱신에 persistent cache나 background refresh를 추가하지 않는다.
   둘 다 freshness와 실패 동작을 바꾸므로 별도의 승인된 설계가 필요하다.

집중 검사:

```bash
cargo test --locked -p yo-provider-openrouter
cargo test --locked -p yo-cli command::connect::external::discovery_tests
cargo test --locked -p yo-cli command::connect::picker
```

## 명시적 무료 이미지 연결

첫 이미지 지원 대상은 아래 NVIDIA Nemotron 3 Nano Omni 무료 모델이다.
다음을 로컬 YAML 파일로 저장하고 `yo connect --from /절대/vision-free.yaml`을
실행한다. 연결 변경안을 확인하고 OpenRouter 키를 입력한다. 일반 확인 화면에도
정확한 모델·endpoint, NVIDIA 전용 가격 상한 0·대체 경로 금지, PNG 이미지 지원과
추정 회계가 표시되므로 저장 전에 확인할 수 있다. 별도 `vision-free`
account 이름은 `default` 연결과 구분하기 위한 것이다. 이미 존재하는 account로
가져오면 해당 모델 그룹 전체를 교체하므로 변경안의 삭제 항목을 확인한다.

```yaml
provider: openrouter
account: vision-free
base_url: https://openrouter.ai/api/v1
profile:
  api_dialect: openai-chat-completions
  tokenizer_profile: utf8-bytes/v1
  input_token_limit: 256000
  max_output_tokens: 65536
  reasoning_parameters: {}
  optional_request_parameters:
    provider:
      only: [nvidia]
      allow_fallbacks: false
      require_parameters: true
      max_price: {prompt: 0, completion: 0, request: 0, image: 0}
  tool_capability_policy: local-tools/v1
  replay_profile: semantic-only/v1
  image_input_profile: openrouter-free-png-advisory/v1
models:
  - model: nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free

```

`/model`에서 새 연결을 고르거나 다음과 같이 시작한다.
`yo --model openrouter:vision-free:nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free`
기존 Ctrl+V 방식으로 이미지를 붙여넣고 미리보기를 확인한 뒤 Enter로 제출한다.
SSH·tmux의 클립보드 원본은 [이미지 입력 흐름](../../architecture/runtime-flow.md#클립보드-획득과-ssh-전달)에
따라 명시적으로 설정한다. 실행 호스트의 파일은 `/attach /절대/image.png`로 고를 수 있다.

전체 프로필이 일치해야 하며 기존 연결에 자동 추가하지 않는다. 텍스트 요청,
도구 결과, 재개, 요약에서도 NVIDIA 경로와 가격 상한 0을 유지하고 대체 경로를
허용하지 않는다. 용량·가격 제한은 실패로 표시하며 자동 재전송하지 않는다.
다른 OpenRouter 모델의 기존 텍스트 동작은 유지한다.

저장한 세션에는 정규화된 PNG 원본과 순서가 남는다. 컨텍스트 표시는 이미지 없는
직렬화 UTF-8 바이트 수에 이미지 occurrence당 2000, 이미지가 있으면 요청당 여유분
1024를 더한 추정값이다. 실제 사용량이나 보장된 상한이 아니며 이를 표시한다.
기존 이미지 입력의 snapshot·원본 크기·요약 한도를 그대로 적용한다.
core의 전체 연결 검증, 중립 Chat Completions 전송, managed 전체 요청 계산이
각 책임을 맡는다.
