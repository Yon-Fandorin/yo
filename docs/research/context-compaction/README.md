# Coding-agent context compaction research

> Status: non-authoritative research input
>
> 조사 기준일: 2026-08-29

이 디렉터리는 장시간 실행되는 코딩 에이전트가 context window 압력에
대응하는 방식을 비교한다. 제품 계약이나 Yo의 동작을 정의하지 않는다.
Upstream 구현은 조사 기준일 이후 바뀔 수 있으며, Yo의 승인된 동작은
Methexis KnowledgeUnit만이 소유한다.

## 조사 대상

- [OpenAI Codex와 Responses API](./codex.md)
- [Pi coding agent](./pi.md)
- [Kimi Code](./kimi-code.md)
- [Qwen Code](./qwen-code.md)
- [Grok Build](./grok-build.md)

각 문서는 다음 질문에 같은 순서로 답한다.

1. 언제 압축을 시작하는가?
2. 어느 경계에서 오래된 문맥과 최근 문맥을 나누는가?
3. 요약 요청에는 무엇이 들어가며 어떤 모델을 사용하는가?
4. TODO, 파일, 도구 결과와 실행 상태는 어떻게 보존하는가?
5. 압축 결과를 어떻게 영속화하고 다음 요청을 복원하는가?
6. 실패, overflow와 반복 압축은 어떻게 처리하는가?
7. Yo가 채택하거나 의도적으로 제외할 부분은 무엇인가?

## 한눈에 보는 비교

| 도구 | 주된 시작 조건 | 최근 원문 보존 | 영속 표현 | 두드러진 특성 |
|---|---|---|---|---|
| Codex / Responses | 설정된 rendered-token threshold | Provider가 관리 | 불투명한 encrypted compaction item | Provider 추론 상태를 포함한 연속성 |
| Pi | `window - reserveTokens` 초과 | 기본 약 20K token | summary와 `firstKeptEntryId` | 투명하고 단순한 summary + suffix |
| Kimi Code | 기본 85%, 고정 reserve 조건 병용 | 최근 prompt와 상태 재주입 | summary와 compaction metrics | overflow 복구와 TODO 보강 |
| Qwen Code | warning / auto / hard의 다단계 threshold | 선택적 최근 결과와 복원 attachment | summary와 복원용 상태 | LLM 전에 deterministic micro-compaction |
| Grok Build | 기본 85% | 구조화된 현재 상태 | summary checkpoint와 transcript | 실행 상태와 대화 요약의 분리 |

숫자가 비슷해 보여도 같은 정책은 아니다. 어떤 도구는 Provider가 보고한
사용량을 쓰고, 어떤 도구는 로컬 추정치를 쓰며, 어떤 도구는 system prompt와
tool schema까지 포함한 rendered input을 기준으로 삼는다. 따라서 85%나 95%라는
숫자만 복사해서는 동일한 안전 여유가 생기지 않는다.

## 공통 구조

다섯 도구에서 반복해서 나타나는 구조는 다음과 같다.

1. context가 가득 차기 전에 압력 상태를 판단한다.
2. 현재 사용자 입력과 최근 작업 구간은 가능한 한 원문으로 남긴다.
3. 오래된 구간을 LLM summary 또는 Provider-native state로 치환한다.
4. tool call/result처럼 의미가 연결된 항목은 함께 다룬다.
5. TODO, 수정 파일과 실행 중 작업처럼 summary에서 빠지기 쉬운 상태를 별도로
   보강한다.
6. 전체 transcript나 Journal은 압축된 모델 입력과 별개로 보존한다.
7. 압축 전후 token과 압축 횟수를 관측 가능하게 만든다.

차이는 주로 세 지점에 있다.

- **투명성:** 사람이 읽을 수 있는 summary인가, Provider만 해석하는 opaque
  item인가.
- **정확성:** 최종 wire payload를 정확히 세는가, 메시지나 문자를 추정하는가.
- **손실 범위:** 오래된 대화만 요약하는가, tool output과 media도 먼저 제거하는가.

## Yo에 대한 분석

Yo의 첫 구현에는 Pi의 투명한 `summary + retained suffix`가 가장 가까운
출발점이다. 여기에 Yo가 이미 가진 Connector 최종 입력의 정확 계측과
anchored Session Journal의 원자적 전환을 결합하면 Pi의 추정 오차와 복구
모호성을 줄일 수 있다.

Kimi Code와 Grok Build에서 참고할 부분은 summary 자체보다 **현재 작업 상태를
명시적으로 보존하는 방식**이다. Qwen Code의 deterministic micro-compaction은
비용 절감 효과가 크지만 tool output을 바꾸는 별도 손실 정책이므로 첫 구현에
몰래 포함해서는 안 된다. Codex의 opaque compaction은 OpenAI Connector 전용
최적화 후보일 수 있지만 Provider-neutral Journal 표현을 대체할 수 없다.

### 첫 구현에 맞는 범위

- Connector가 만든 전체 입력을 정확히 센다.
- 90% 이상에서 persisted context strategy가 `Compact` 또는 `Reject`를 결정한다.
- 오래된 visible semantic prefix만 같은 binding으로 한 번 요약한다.
- tools를 비활성화하고 최근의 완전한 semantic group과 현재 입력을 원문으로
  보존한다.
- rebuilt payload를 다시 정확히 센 뒤 `Admit`일 때만 Journal epoch를 원자적으로
  전환한다.
- 실패, 취소 또는 두 번째 `Compact`에서는 원 Journal을 변경하지 않는다.

### 독립된 후속 계약이 필요한 후보

- 오래된 tool output이나 image를 규칙 기반으로 지우는 micro-compaction
- 더 저렴한 별도 compaction model 선택
- Provider-native opaque compaction item
- 현재 파일을 다시 읽어 과거 context 대신 넣는 restoration attachment
- 두 번 이상의 요약, retry 또는 fallback
- 모델이 원 transcript를 필요할 때 읽는 transcript-pointer tool

## Yo의 승인된 소유자

이 조사보다 아래 KnowledgeUnit이 우선한다.

- [`agent.backend.yo-managed-model-loop`](../../../methexis/knowledge/agent-runtime/agent.backend.yo-managed-model-loop.md)
- [`agent.persistence.format-compatibility`](../../../methexis/knowledge/agent-runtime/agent.persistence.format-compatibility.md)
- [`agent.session.continuation-lineage`](../../../methexis/knowledge/agent-runtime/agent.session.continuation-lineage.md)

리서치에서 새로운 선택지를 발견하더라도 위 계약을 구현 중에 암묵적으로
확장하지 않는다. 별도 제품 결정을 거쳐 해당 소유자를 갱신해야 한다.
