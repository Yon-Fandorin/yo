# Kimi Code compaction

> Status: non-authoritative research input
>
> 조사 기준일: 2026-08-30

## 전체 구조

Kimi Code의 compaction은 긴 history를 한 번에 요약하는 full compaction과 오래된
대형 tool 결과를 먼저 줄이는 micro compaction으로 발전해 왔다. full compaction은
사용자가 요청할 수도 있고 agent turn의 안전한 step boundary에서 자동으로
시작될 수도 있다.

수동 압축은 진행 중인 model turn과 경쟁하지 않도록 idle 상태에서 수행한다. 자동
압축은 tool call과 result 사이가 아니라 한 step이 끝난 뒤 판단해 현재 실행
중인 작업의 인과관계를 보존한다.

## 시작 조건

조사한 strategy 기본값에는 다음 정책이 있었다.

- trigger threshold: context 사용률 85%
- block threshold: 85%
- reserved context size: 50,000 token
- 한 turn 안의 최대 compaction 횟수: 제한 없음
- Provider overflow recovery: 연속 최대 3회

단순 비율뿐 아니라 `used + reservedContextSize >= maxContextSize`도 판단에
사용한다. 작은 context model에서는 고정 reserve가 비율보다 먼저 작동할 수 있고,
큰 context model에서는 85%가 먼저 작동할 수 있다.

Kimi Code는 local token estimation도 사용한다. Provider usage가 없거나 request가
실패한 상황에서도 압력 판단을 계속하려는 목적이다. changelog에는 Provider의
413/context overflow를 감지해 observed maximum을 낮추고 다시 압축하는 복구가
추가된 기록도 있다.

## Full compaction

Full compaction은 오래된 conversation을 LLM에 전달해 continuation용 summary를
만든다. 구현과 changelog에서 확인되는 주요 의도는 다음과 같다.

- 최근 사용자 prompt는 가능한 한 별도로 유지한다.
- 오래된 assistant/tool history는 summary로 치환한다.
- summary 뒤에 TODO list를 붙여 진행 상태 손실을 줄인다.
- 압축 후 system prompt와 reminder/state를 새 context에 다시 넣는다.
- tool call과 result adjacency가 깨지지 않도록 history를 정리한다.

과거 구현 개편에서는 한 개의 user-role summary와 최근 user prompt를 중심으로
새 context를 구성하고 오래된 assistant/tool message를 제거했다. 이후 release에서
summary 표시, state handoff와 micro compaction이 계속 보강됐다. 따라서 특정
버전의 message shape를 Kimi Code의 영구 계약으로 간주해서는 안 된다.

### 현재 handoff 출력 형식

조사 기준일의 prompt는 rigid section heading을 쓰지 말라고 명시하고, 현재 대화와
같은 언어로 1인칭 현재형 handoff를 작성하게 한다. 다음 turn에는 최근 user
message와 이 note만 남는다고 가정한다.

Handoff에는 최신 요청의 실제 의도, 계속 적용되는 제약과 결정, 검증된 command와
path 및 결과, 아직 모르는 사실, 정확한 다음 command와 남은 순서를 담는다. 완료를
검증하지 않은 작업은 명시적으로 unverified로 남긴다. Live TODO는 자동으로 다시
붙으므로 목록 자체를 복제하지 않고 task 사이의 이유와 순서만 기록한다.

구현은 assistant/tool history를 버리고 summary와 최근 실제 user message를
user-role context로 재구성한다. 최근 message budget은 약 20K token이며 앞쪽 2K와
뒤쪽 18K를 우선하여 중간이 생략될 수 있다. 따라서 자유형 note만 보고 모든 원문이
남는다고 가정할 수 없다.

## Micro compaction

최근 Kimi Code는 전체 LLM summary 전에 오래되고 큰 tool result를 줄이는 경로를
사용한다. 목적은 다음과 같다.

- source file 전체 내용이나 긴 command output이 context 대부분을 차지하는 상황을
  싸게 완화한다.
- 중요한 최근 tool result는 그대로 남긴다.
- full compaction 호출 빈도와 summary 비용을 줄인다.

이는 자연어 summary보다 빠르고 결정적일 수 있지만, model이 이전에 봤던 정확한
tool output이 다음 request에서는 사라질 수 있다는 별도의 loss policy다.

## 실패와 관측

Kimi Code는 다음 정보를 compaction trace/metrics에 남긴다.

- 압축 전후 token
- compacted message와 dropped message 수
- retry 횟수
- summary request의 usage
- compaction 종류와 원인

Provider가 예상보다 작은 context limit으로 overflow를 반환하면 limit 관찰값을
보정하고 압축을 반복할 수 있다. 실사용 복구에는 유용하지만 잘못된 summary가
계속 누적되거나 한 turn이 compaction loop에 머무를 위험이 있다. 연속 overflow
attempt 상한은 이 문제를 완화하지만 exact-once는 아니다.

## Yo에 대한 적용 판단

가져올 점:

- 자동 압축을 tool execution 중간이 아닌 완전한 semantic boundary에서 수행한다.
- TODO, reminder와 현재 작업 상태를 summary 뒤에 명시적으로 보강한다.
- compaction 전후 token, dropped count와 이유를 usage와 별도로 관측한다.
- Provider가 보고하는 context limit과 catalog limit이 다를 수 있음을 진단 정보로
  남긴다.

첫 구현에서 제외할 점:

- 고정 50,000-token reserve
- 한 turn에서 제한 없는 compaction
- Provider overflow 후 최대 세 번 재시도
- full summary와 micro-compaction을 하나의 손실 계약으로 합치는 것
- Provider limit 오류를 근거로 persisted model binding을 조용히 변경하는 것

Yo에서는 summary request를 같은 binding에서 tools 없이 정확히 한 번만 수행하고,
rebuilt payload가 다시 `Compact`라면 typed `Reject`로 끝내는 승인 계약이 더
예측 가능하다. Kimi의 micro compaction은 효과가 확인되더라도 별도 versioned
strategy로 설계해야 한다.

## 출처

- [Kimi Code full compaction implementation](https://github.com/MoonshotAI/kimi-code/blob/main/packages/agent-core/src/agent/compaction/full.ts)
- [Kimi Code compaction strategy](https://github.com/MoonshotAI/kimi-code/blob/main/packages/agent-core/src/agent/compaction/strategy.ts)
- [Kimi Code handoff instruction](https://github.com/MoonshotAI/kimi-code/blob/main/packages/agent-core/src/agent/compaction/compaction-instruction.md)
- [Kimi Code handoff history construction](https://github.com/MoonshotAI/kimi-code/blob/main/packages/agent-core/src/agent/compaction/handoff.ts)
- [Kimi Code changelog](https://github.com/MoonshotAI/kimi-code/blob/main/apps/kimi-code/CHANGELOG.md)
- [Kimi Code: What's new](https://www.kimi.com/code/docs/en/kimi-code/whats-new.html)
