# OpenAI Codex and Responses compaction

> Status: non-authoritative research input
>
> 조사 기준일: 2026-08-30

## 조사 범위

공개적으로 확인할 수 있는 안정된 계약은 Codex 내부 구현보다 OpenAI Responses
API의 compaction 기능이다. 이 문서는 공식 API 동작과 현재 로컬에 설치된 Codex
CLI에서 관찰한 값을 구분한다. 로컬 binary의 symbol이나 bundled model catalog는
해당 설치본의 증거일 뿐 OpenAI 제품 전체의 공개 계약이 아니다.

## 서버 측 compaction

Responses API는 request의 `context_management`에 compaction 설정과 token
threshold를 전달할 수 있다. 서버가 실제로 렌더링한 입력 token이 threshold를
넘으면 response stream에 compaction item이 생긴다.

이 item은 다음 성질을 갖는다.

- encrypted하고 opaque하므로 client가 내부 summary를 읽거나 수정하지 않는다.
- 이전 대화뿐 아니라 다음 reasoning에 필요한 Provider-side 상태를 전달한다.
- 다음 request의 input에 포함해 압축된 상태에서 계속할 수 있다.
- 여러 차례 반복해 사용할 수 있다.
- stateless input 배열을 직접 관리한다면 최근 compaction item 이전의 오래된
  항목을 제거해 request 크기를 줄일 수 있다.
- `previous_response_id`로 stateful continuation을 사용한다면 client가 같은 방식으로
  과거 input을 수동 정리하지 않는다.

별도의 `/responses/compact` 경로는 명시적 compaction을 제공한다. 공식 API
reference는 compacted response에 사용자 메시지와 하나의 opaque compaction item이
포함되는 형태를 설명한다. 이 결과 역시 inspection용 summary가 아니라 다음
request로 전달할 continuation state다.

## Token과 비용 관측

공식 compaction guide는 Responses usage가 input, output, reasoning, cached token을
보고하며 compaction request도 사용량을 발생시킨다고 설명한다. 따라서 사용자가
보는 context 감소량과 청구 또는 누적 usage는 같은 숫자가 아니다.

- compaction 후 다음 request의 visible input은 줄 수 있다.
- compaction을 만드는 request 자체에는 input과 output usage가 있다.
- cached input은 새로 계산한 token과 비용 의미가 다르더라도 total input usage에
  포함될 수 있다.
- opaque item의 byte 크기만으로 내부에 보존된 의미량을 판단할 수 없다.

## Local summary 형식과 retained history

`codex-cli 0.150.1`의 정확한 `rust-v0.150.1` source에서 local fallback prompt와
history replacement를 교차 확인했다. Prompt는 다음 종류의 정보를 요구하지만
고정 Markdown heading이나 XML schema는 강제하지 않는다.

- 현재 진행과 핵심 결정
- 계속 적용되는 context, 제약과 사용자 선호
- 남은 작업과 명확한 다음 단계
- 작업을 재개하는 데 필요한 값, 예시와 참조

결과는 마지막 assistant text를 fixed compaction prefix와 함께 사용한다. Local
replacement는 최근 user message를 최대 약 20K token까지 summary와 함께 남기며,
경계 message text를 줄일 수 있다. 반면 remote v2는 visible summary가 아니라
Provider가 만든 opaque item과 허용된 최근 message를 사용한다. 두 경로를 하나의
사람이 읽는 format으로 설명하면 안 된다.

## 현재 로컬 Codex 관찰

조사 환경의 `codex-cli 0.150.1`을 대상으로 다음을 확인했다.

```text
gpt-5.6-sol: context_window 272000, max_context_window 872000,
             effective_context_window_percent 95
gpt-5.4:     context_window 272000, max_context_window 1000000,
             effective_context_window_percent 95
```

bundled catalog에는 두 모델 모두 token 기반 truncation policy도 기록돼 있었다.
Binary의 모듈 이름에서는 remote compaction v2와 local fallback 경로가 함께
관찰됐다. 다음 사항은 공개 문서만으로 확정할 수 없다.

- 어떤 조건에서 CLI가 remote와 local compaction을 선택하는가.
- `effective_context_window_percent`가 모든 모델과 계정에서 같은가.
- Provider-native item에 어떤 private reasoning state가 보존되는가.

따라서 이 값들은 Yo의 기본 threshold나 공개 모델 계약으로 복사하면 안 된다.

## 장점과 한계

### 장점

- Provider가 이해하는 reasoning state까지 이어 갈 수 있다.
- client가 긴 summary prompt와 복원 schema를 직접 관리하지 않아도 된다.
- stateful Responses continuation과 자연스럽게 결합된다.
- opaque payload가 client에 reasoning 원문을 노출하지 않는다.

### 한계

- 사람이 summary의 정확성과 손실 내용을 직접 검토할 수 없다.
- Provider가 바뀌면 같은 item을 재생할 수 있다고 기대할 수 없다.
- client Journal이 opaque bytes만 보존하면 semantic recovery를 독립 검증하기
  어렵다.
- Provider-private state가 삭제됐는지, 축약됐는지 client가 정확히 기술하기
  어렵다.

## Yo에 대한 적용 판단

Codex 방식은 Yo의 Provider-neutral 기본 compaction을 대체하기보다 향후
OpenAI Connector capability로 다루는 편이 맞다.

첫 구현에서 가져올 점은 다음과 같다.

- wire/rendered input 기준으로 compaction 시점을 판단한다.
- context가 완전히 찬 뒤 복구하기보다 여유가 남았을 때 선제 수행한다.
- compaction을 한 번의 continuation transition으로 취급하고 usage를 별도로
  관측한다.

첫 구현에서 제외할 점은 다음과 같다.

- opaque item을 공용 Journal summary로 사용하는 것
- OpenAI remote compaction availability에 Session 복구를 의존하는 것
- Codex 설치본의 95% 값을 모든 Provider에 적용하는 것

장래에 native capability를 추가한다면 visible UTF-8 handoff와 opaque Provider
state를 서로 다른 필드와 loss class로 보존해야 한다. opaque state가 없어도
Journal의 visible history가 독립 복구 가능해야 한다.

## 출처

- [OpenAI Docs: Compaction](https://developers.openai.com/api/docs/guides/compaction)
- [OpenAI API reference: Compact a response](https://developers.openai.com/api/reference/java/resources/responses/methods/compact)
- [OpenAI Docs: Latest model guidance](https://developers.openai.com/api/docs/guides/latest-model?model=gpt-5.2)
- [Codex local compaction prompt `rust-v0.150.1`](https://github.com/openai/codex/blob/rust-v0.150.1/codex-rs/prompts/templates/compact/prompt.md)
- [Codex local compaction implementation `rust-v0.150.1`](https://github.com/openai/codex/blob/rust-v0.150.1/codex-rs/core/src/compact.rs)
- [Codex remote compaction v2 `rust-v0.150.1`](https://github.com/openai/codex/blob/rust-v0.150.1/codex-rs/core/src/compact_remote_v2.rs)
- 로컬 관찰: `codex-cli 0.150.1`, `codex debug models --bundled`
