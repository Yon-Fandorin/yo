# Kimi 카탈로그와 Connector 관리

Kimi는 소수의 검토된 실행 overlay를 함께 쓰는 인증된 runtime-discovery
Provider다. 계정 inventory는 계속 표시하지만 Yo가 완전한 request, stream,
limit, replay 동작을 아는 행만 선택할 수 있다. overlay를 추측한 모델 계열
이름 allowlist로 바꾸지 않는다.

## 공식 출처

Kimi의 공식 Platform API와 모델 가이드를 사용한다.

- request와 streaming response 형태는
  [Chat API](https://platform.kimi.ai/docs/api/chat)를 사용한다.
- K3 limit와 reasoning 설정은
  [Kimi K3 quickstart](https://platform.kimi.ai/docs/guide/kimi-k3-quickstart)와
  [reasoning effort](https://platform.kimi.ai/docs/guide/use-reasoning-effort)를
  사용한다.
- tool round 사이의 완전한 assistant-message replay는
  [Kimi K3 tool calling](https://platform.kimi.ai/docs/guide/kimi-k3-tool-calling-best-practice)을
  사용한다.

인증된 `GET https://api.moonshot.ai/v1/models` 결과는 Account의 현재
inventory를 증명한다. 그것만으로 안전한 Yo 실행 profile이 정해지는 것은
아니다. 모든 응답 byte를 신뢰하지 않는 bounded 입력으로 취급한다.

## 코드 소유 경계

| 책임 | 소유자 |
|---|---|
| Account seed, bounded discovery transport, normalization, 검토된 overlay, typed disabled reason | [`kimi_catalog.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/kimi_catalog.rs)와 `kimi_catalog/` 하위 모듈 |
| 정확한 Kimi request와 streamed assistant-message 문법 | [`kimi_request.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_connector/kimi_request.rs), [`connector.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_connector/connector.rs), [`chat_sse.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_connector/chat_sse.rs) |
| Provider-private replay 검증, 저장, 상관관계, native 재사용 | [`backend/evidence/replay.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/evidence/replay.rs), [`journal/codec`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/journal/codec), [`backend/native`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/native/mod.rs) |
| Config seed, picker, disclosure, 검증된 연결 transaction | [`config.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/config.rs), [`external.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection/external.rs), [`picker.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection/input/picker.rs) |

## 현재 갱신 절차

1. bounded 인증 `/models` 응답을 캡처해 모든 유효한 exact ModelId를 현재
   overlay와 비교한다. 첫 유효 duplicate가 이기며 4,097개 이상의 행은
   snapshot 전체를 거부한다.
2. unknown, retired, capability-conflicting 행은 typed 이유와 함께 표시하되
   비활성화한다. complete connector envelope와 replay 동작이 승인된 뒤에만
   선택 가능한 행을 추가한다.
3. context 근거를 독립적으로 다시 확인한다. K3는 1,048,576까지의 양의 remote
   값을, 검토된 K2.7과 K2.6 행은 262,144까지의 양의 값을 사용할 수 있다.
   검토 범위를 벗어난 remote 값은 로컬 허용 범위를 넓히지 않고 행을
   비활성화한다.
4. exact replay 경계를 보존한다. K3와 검토된 K2.7 coding variant 두 개는
   `kimi-private-local-plaintext/v1`이 필요하고 K2.6은
   `semantic-only/v1`을 유지한다. ModelId나 connector에서 동의를 추론하지
   않는다.
5. managed private-replay binding을 게시하기 전에 연결 preview가 bounded Kimi
   assistant state를 현재 사용자 로컬 Session 기록에 암호화하지 않고 보관한다고
   계속 알리는지 검증한다.
6. 완전한 tool round 하나를 실행한다. visible assistant/function projection과
   provider-private assistant 항목 하나가 atomically 저장되고 같은 binding epoch와
   상관되며, visible content나 tool call을 중복하지 않고 다음 Kimi request를
   재구성해야 한다.

집중 검사:

```bash
cargo test --locked -p yo-core kimi
cargo test --locked -p yo-core journal::codec::tests::correlation::continuation
cargo test --locked -p yo-core backend::native
cargo test --locked -p yo-cli kimi
```

credential, raw private reasoning, live account response를 저장소, review packet,
runbook에 보관하지 않는다. 행 갱신은 이전에 저장된 managed binding, Session
record, 동의 결정을 다시 쓰지 않는다.
