# 새 Provider 추가

Kimi와 이후 모든 Provider에 이 페이지를 사용한다. 새 Provider는 단순히 모델
목록 하나를 추가하는 일이 아니다. Source authority, endpoint·protocol 의미,
credential verification, profile resolution, durable-state compatibility가 함께
생긴다.

## 출처 감사 완료하기

구현을 선택하기 전에 공식 출처로 다음 질문에 답한다.

1. 모델 출처가 public documentation, public API, authenticated account-scoped
   API 중 무엇인가?
2. Global product list, subscription plan, 이 credential로 실제 사용할 수 있는
   모델 중 무엇을 설명하는가?
3. Stable ModelId와 endpoint, dialect, modality, tool, reasoning behavior,
   limit을 제공하는가, 아니면 marketing description만 제공하는가?
4. 모델을 나열하고 검증할 때 어떤 authentication material이 필요한가?
5. Region, plan, protocol variant는 별도 catalog profile인가?
6. Removal, deprecation, 같은 ModelId의 field 변경을 어떻게 알리는가?

Kimi는 공식
[Kimi model list](https://platform.kimi.ai/docs/models)와 공식 endpoint 또는
protocol 문서부터 조사한다. Authenticated list operation이 account-scoped이며
충분한 typed data를 제공하는지 확인한다. Kimi가 OpenRouter의 dynamic 설계나
QwenCloud의 static 설계를 그대로 따라야 한다고 미리 가정하지 않는다.

## 가장 작은 안전한 제품 형태 선택하기

- Authenticated official source가 현재 account inventory와 complete binding에
  필요한 필드를 안전하게 증명할 때만 runtime discovery를 선택한다.
- 공식 exact allowlist와 stable profile 의미가 있지만 account-scoped
  discovery가 없을 때 static registry를 선택한다.
- 어느 출처도 충분하지 않으면 explicit manual binding을 유지한다. 편리한
  목록이 capability나 entitlement를 지어낼 이유는 아니다.

활성 model-service binding 계약이 선택한 source, profile, availability,
refresh 동작을 아직 다루지 않으면 구현보다 먼저 별도 SOT-first contract
Slice와 activation을 완료한다.

## Provider 전용 경계 만들기

`yo-core/src/model_service` 아래에 하나의 응집된 Provider module을 만든다.
Transport와 normalization의 책임이 실제로 다를 때만 submodule로 나눈다.
Provider-neutral catalog entry, complete binding, picker, verification, journal,
connection transaction을 재사용한다. Typed adapter가 같은 handoff를 만들 수
있다면 shared layer에 Provider branch를 추가하지 않는다.

`docs/src/workflows/provider-catalogs/<provider>.md`를 만들고 다음을 기록한다.

- 공식 source link와 각 출처가 증명하는 것
- static 또는 dynamic 분류와 이유
- Code owner별 accepted profile name, endpoint, regional boundary
- 정확한 집중 검증 명령
- Deprecation·refresh 절차
- 일반 baseline에서 실행할 수 없는 environment check

이 사실들은 공통 가이드와 다른 Provider 런북에 넣지 않는다. 대응하는 한국어
Projection을 추가하고 의미 검토 뒤 canonical source hash를 승인한다.

## Acceptance evidence

첫 Provider Slice는 sample response 하나를 parse하는 데 그치지 않고 happy
path와 counterexample을 함께 증명해야 한다.

- Exact configured Provider와 Account가 의도한 catalog owner를 선택한다.
- Cancellation은 secret 입력과 repository mutation보다 먼저 일어난다.
- 선택된 complete binding이 기존 verified connection transaction에 들어간다.
- Unsupported row는 표시되지만 선택할 수 없다.
- Malformed, duplicate, oversized, redirected, stale, incomplete input이 자기 소유
  boundary에서 실패한다.
- Diagnostic과 evidence에 secret이나 private credential revision이 들어가지
  않는다.
- Startup과 recovery가 exact durable binding을 계속 사용한다.
- 이후 discovery에서 제거돼도 기존 managed state를 삭제하거나 조용히
  교체하지 않는다.
