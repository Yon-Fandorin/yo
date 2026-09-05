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
| 크기가 제한된 authenticated transport | [`openrouter_discovery/transport.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/openrouter_discovery/transport.rs) |
| Response parsing, normalization, availability, authored override | [`openrouter_discovery.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/openrouter_discovery.rs)와 [`openrouter_discovery/normalize.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/openrouter_discovery/normalize.rs) |
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
cargo test --locked -p yo-core openrouter_discovery
cargo test --locked -p yo-cli command::connect::external::discovery_tests
cargo test --locked -p yo-cli command::connect::picker
```
