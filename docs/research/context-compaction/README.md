# Coding-agent context compaction research

> Status: non-authoritative research input
>
> 조사 기준일: 2026-08-30

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

## Summary 출력 형식 비교

도구들은 같은 정보를 보존하려 해도 모델에게 요구하는 출력 형식은 서로 다르다.

| 도구 | 모델 생성 본문 | 런타임이 별도로 보강하는 내용 |
|---|---|---|
| Codex local | 진행, 결정, 제약, 다음 단계와 핵심 참조를 담은 자유형 handoff | 최근 사용자 메시지와 고정 continuation prefix |
| Codex remote | client가 해석하지 않는 opaque compaction item | Responses continuation 상태 |
| Kimi Code | 대화 언어의 1인칭 현재형 handoff, 고정 heading 없음 | 최근 사용자 prompt, live TODO와 summary prefix |
| Pi | `Goal`, `Constraints & Preferences`, `Progress`, `Key Decisions`, `Next Steps`, `Critical Context`의 고정 Markdown | 읽거나 수정한 파일 목록과 retained suffix |
| Qwen Code | `<state_snapshot>` 아래 고정 XML field | 복원 attachment와 현재 파일·상태 관찰 |
| Grok Build | `<summary>` 안의 번호 section 또는 짧은 자유형 summary | 마지막 실제 query, task, sub-agent, TODO, MCP, project instruction과 transcript pointer |

자유형 handoff는 자연스럽고 task 크기에 비례하기 쉽지만 구조 누락을 기계적으로
찾기 어렵다. 엄격한 XML은 검증하기 쉽지만 모든 사용자 메시지나 전체 code snippet
복제를 요구하면 summary 자체가 다시 커진다. 고정 Markdown은 두 극단의 중간이며,
사람과 모델이 읽기 쉽고 heading 존재와 순서를 결정적으로 검사할 수 있다.

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

### 조사 결과를 조합한 첫 portable 형식

Yo에는 모델이 고정 Markdown heading 안에서 현재 대화 언어로 자유롭게 쓰고,
기계가 아는 값은 모델이 다시 생성하지 않는 혼합형이 적합하다.

```text
# Context Checkpoint
## Current Objective
## Active Constraints
## Decisions
## Verified Progress
## Current State
## Unknown or Unverified
## Next Actions
## Critical References
```

이 본문은 exact metadata의 authority가 아니다. `context_epoch`, source event 범위,
retained semantic group, token 측정, loss disclosure, content-addressed receipt와 hash는
Journal이 구조화된 envelope로 덧붙여야 한다. live TODO나 structured plan이 별도
authority로 존재하면 summary가 그 목록을 복제하지 않고, 다음 작업의 이유와 순서만
남긴다. 근거가 없는 섹션은 내용을 발명하거나 비워 두지 않고 exact `None.`으로
표시한다. 모든 사용자 메시지, 전체 code snippet, credentials, private reasoning과
provider-private bytes는 본문에 넣지 않는다.

### 첫 구현에 맞는 범위

- Connector가 만든 전체 입력을 정확히 센다.
- 기본 85%에서 warning을 관측하고 90% 이상에서 persisted context strategy가
  `Compact` 또는 `Reject`를 결정한다.
- pressure decision은 exact `Admit`, `Compact`, `Reject` 세 가지이고 warning은
  별도 관측 상태다. 압축 후 두 번째 `Compact`는 typed `Reject`로 닫는다.
- 오래된 visible semantic prefix만 같은 binding으로 한 번 요약한다.
- tools를 비활성화하고 현재 입력, 미처리 steer와 approval, 최신 complete semantic
  group, canonical system/tool context를 원문으로 보존한다.
- 추가로 보존하는 과거 원문의 budget은 input limit의 10%와 65,536 token 중 작은
  값으로 제한한다. 현재 입력, 미처리 steer·approval, 최신 complete semantic group과
  canonical system/tool context는 이 선택 예산 때문에 자르지 않는다.
- 필수 보존 집합과 summary만으로 rebuilt payload가 trigger에 도달하면 압축 성공으로
  간주하지 않고 `Reject`한다.
- rebuilt payload를 다시 정확히 센 뒤 `Admit`일 때만 별도 `context_epoch`을
  원자적으로 전환한다. Backend binding epoch은 바뀌지 않는다.
- active Turn에서는 완결된 tool/approval semantic group 뒤와 다음 ordinary Turn
  model request
  사이에서만 checkpoint를 만든다. checkpoint가 exact system/tools 계약을 소유하고,
  Turn 종료 delta는 최신 checkpoint 뒤의 model-visible suffix만 기록한다.
- 실패, 취소 또는 두 번째 `Compact`에서는 원 Journal을 변경하지 않는다.
- summarized prefix의 오래된 대형 tool/media output은 raw Journal을 지우지 않고
  content-addressed receipt를 operator disclosure로 남긴다. receipt나 placeholder가
  model input을 치환하지 않으며, model-visible bounded read는 frozen tool registry를
  바꾸는 별도 후속 계약으로 둔다.
- 자동 압축과 `/compact`는 같은 pipeline과 실패 규칙을 사용한다.

### 독립된 후속 계약이 필요한 후보

- 오래된 tool output이나 image를 규칙 기반으로 지우는 micro-compaction
- 더 저렴한 별도 compaction model 선택
- Provider-native opaque compaction item
- 현재 파일을 다시 읽어 과거 context 대신 넣는 restoration attachment
- 두 번 이상의 요약, retry 또는 fallback
- Provider나 Session 경계를 넘는 transcript-pointer 또는 임의 path read
- receipt를 검증해 Session Journal 원문 일부를 읽는 bounded artifact tool

## Yo의 승인된 소유자

이 조사보다 아래 KnowledgeUnit이 우선한다.

- [`agent.backend.yo-managed-model-loop`](../../../methexis/knowledge/agent-runtime/agent.backend.yo-managed-model-loop.md)
- [`agent.persistence.format-compatibility`](../../../methexis/knowledge/agent-runtime/agent.persistence.format-compatibility.md)
- [`agent.session.continuation-lineage`](../../../methexis/knowledge/agent-runtime/agent.session.continuation-lineage.md)

리서치에서 새로운 선택지를 발견하더라도 위 계약을 구현 중에 암묵적으로
확장하지 않는다. 별도 제품 결정을 거쳐 해당 소유자를 갱신해야 한다.
