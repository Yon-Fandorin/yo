# Kimi 카탈로그와 Connector 관리

Kimi는 Platform AI와 Code Membership 제품을 분리한 인증된 runtime-discovery
Provider다. 각 제품은 소수의 검토된 실행 overlay를 쓴다. 계정 inventory는
계속 표시하지만 Yo가 완전한 request, stream, limit, replay 동작을 아는 행만
선택할 수 있다. 어느 overlay도 추측한 모델 계열 이름 allowlist로 바꾸거나 한
제품의 endpoint, entitlement, request policy를 다른 제품에 섞지 않는다.

## 공식 출처

Kimi의 공식 제품별 API와 모델 가이드를 사용한다.

- request와 streaming response 형태는
  [Chat API](https://platform.kimi.ai/docs/api/chat)를 사용한다.
- K3 limit와 reasoning 설정은
  [Kimi K3 quickstart](https://platform.kimi.ai/docs/guide/kimi-k3-quickstart)와
  [reasoning effort](https://platform.kimi.ai/docs/guide/use-reasoning-effort)를
  사용한다.
- tool round 사이의 완전한 assistant-message replay는
  [Kimi K3 tool calling](https://platform.kimi.ai/docs/guide/kimi-k3-tool-calling-best-practice)을
  사용한다.
- Code Membership 모델 ID, context limit, 추천 정보는
  [Kimi Code models](https://www.kimi.com/code/docs/en/kimi-code/models.html)을
  사용한다.
- Code endpoint, preserved-thinking request shape, Session cache affinity는
  [Kimi Code documentation](https://www.kimi.com/code/docs/en/)을 사용한다.

Platform profile은 `https://api.moonshot.ai/v1/`, Code Membership profile은
`https://api.kimi.com/coding/v1/`를 사용한다. 인증된 `GET models` 결과는 해당
제품 Account의 현재 inventory만 증명한다. 그것만으로 안전한 Yo 실행 profile이나
다른 제품의 entitlement가 정해지는 것은 아니다. 모든 응답 byte를 신뢰하지 않는
bounded 입력으로 취급한다.

## 코드 소유 경계

| 책임 | 소유자 |
|---|---|
| 제품별 Account seed, bounded discovery transport, normalization, 검토된 overlay, typed disabled reason | [`kimi_catalog.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/kimi_catalog.rs)와 `kimi_catalog/` 하위 모듈 |
| exact Platform/Code profile admission, Kimi request·stream 문법, typed private payload codec·projection, encoded-size 계산 | [`connectors/kimi`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/connectors/kimi/src/lib.rs) |
| opaque provider-private envelope 제한, physical 저장, replay-profile/schema 상관관계, neutral projection 비교, Provider-neutral Session별 cache-affinity hint 생성 | [`yo-backend evidence/replay.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/foundation/src/evidence/replay.rs), [`yo-core backend/evidence.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/backend/evidence.rs), [`journal/codec`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/journal/codec), [`backends/managed`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/backends/managed/src/lib.rs) |
| Config seed, picker, disclosure, 복구 가능한 연결 transaction | [`config.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/config.rs), [`external.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection/external.rs), [`picker.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection/input/picker.rs) |

## 현재 갱신 절차

1. 먼저 정확히 설정된 catalog profile을 선택한다. Platform은
   `kimi-platform-ai/v1`, Code는 `kimi-code-membership/v1`이다. 해당 profile의
   endpoint에서 bounded 인증 `/models` 응답 하나를 캡처해 모든 유효한 exact
   ModelId를 그 제품의 overlay와만 비교한다. 첫 유효 duplicate가 이기며
   4,097개 이상의 행은 snapshot 전체를 거부한다.
2. unknown, retired, capability-conflicting 행은 typed 이유와 함께 표시하되
   비활성화한다. complete connector envelope와 replay 동작이 승인된 뒤에만
   선택 가능한 행을 추가한다.
3. context 근거를 독립적으로 다시 확인한다. Platform K3는 remote context
   131,073부터 1,048,576까지를, Platform K2.7과 K2.6은 32,769부터
   262,144까지를 양 끝 포함으로 허용한다. Code `k3`는 문서화된 262,144부터
   1,048,576까지의 tier를 허용하고,
   `k3-256k`, `kimi-for-coding`, `kimi-for-coding-highspeed`는 정확히
   262,144를 요구한다. 선택한 제품의 검토 범위를 벗어난 remote 값은 로컬 허용
   범위를 넓히지 않고 행을 비활성화한다.
4. exact replay 경계를 보존한다. 두 제품에서 선택 가능한 K3 또는 K2.7 행은
   모두 `kimi-private-local-plaintext/v1`이 필요하고 Platform K2.6은
   `semantic-only/v1`을 유지한다. ModelId나 connector에서 동의를 추론하지
   않는다.
5. managed private-replay binding을 게시하기 전에 연결 preview가 bounded Kimi
   assistant state를 현재 사용자 로컬 Session 기록에 암호화하지 않고 보관한다고
   계속 알리는지 검증한다.
6. 완전한 tool round 하나를 실행한다. visible assistant/function projection과
   provider-private assistant 항목 하나가 atomically 저장되고 같은 binding epoch와
   상관되며, visible content나 tool call을 중복하지 않고 다음 Kimi request를
   재구성해야 한다.
7. Code에서는 Session 하나가 일반 요청과 재개 요청에서 opaque cache-affinity
   hint 하나를 재사용하는지 확인한다. Connector만 이를 `prompt_cache_key`로
   직렬화한다. Platform과 다른 connector는 무시하고, hint는 binding identity, replay
   evidence, log, diagnostic, Transcript, trace에 들어가지 않는다.

집중 검사:

```bash
cargo test --locked -p yo-connector-kimi
cargo test --locked -p yo-core kimi
cargo test --locked -p yo-core journal::codec::tests::correlation::continuation
cargo test --locked -p yo-core backend::native
cargo test --locked -p yo-cli kimi
```

credential, raw private reasoning, live account response를 저장소, review packet,
runbook에 보관하지 않는다. 행 갱신은 이전에 저장된 managed binding, Session
record, 동의 결정을 다시 쓰지 않는다.
