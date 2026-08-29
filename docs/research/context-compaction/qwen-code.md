# Qwen Code compaction

> Status: non-authoritative research input
>
> 조사 기준일: 2026-08-29

## 다단계 압력 관리

Qwen Code는 하나의 threshold만 두기보다 warning, automatic compaction, hard
limit을 구분한다. 실제 threshold 계산은 model context window에서 summary를
만들 여유를 제외한 effective window와 percentage/absolute ceiling을 함께
고려한다. 설정과 구현에서 기본 auto threshold는 약 85% 계열이다.

단계별 의도는 다음과 같다.

1. warning 구간에서는 사용자와 runtime이 context 압력을 알 수 있다.
2. auto 구간에서는 정상 model request 전에 압축을 시도한다.
3. hard 구간에서는 안전하게 request를 막아 Provider overflow와 무한 복구를
   피한다.

이 구조는 "압축을 시작할 시점"과 "더 이상 정상 요청을 허용하지 않을 시점"을
분리한다는 점에서 유용하다.

## Micro-compaction

Qwen Code는 LLM summary보다 먼저 deterministic micro-compaction을 수행할 수
있다. 주요 대상은 오래된 tool result와 image/media 결과다.

- tool 종류별로 최근 결과 일부는 보존한다.
- 오래된 대형 결과는 placeholder나 축약 표현으로 대체한다.
- 파일과 이미지에는 서로 다른 최근 보존 개수를 적용할 수 있다.
- idle, 크기 threshold 또는 강제 실행 같은 trigger를 구분한다.
- 절약한 token 수와 evicted path를 기록한다.

이 경로는 모델 호출이 없으므로 빠르고 비용이 작다. 특히 같은 파일 전체를 여러
번 읽거나 screenshot이 반복된 session에서 효과가 크다. 반면 제거된 tool result가
후속 추론에 꼭 필요했는지 자연어 의미로 판단하지 못한다.

## LLM summary와 side query

Micro-compaction 뒤에도 context가 크면 chat compression service가 별도 summary
query를 실행한다. 현재 구현은 다음을 고려한다.

- 남은 context와 model output 한도에서 summary output budget 계산
- main conversation과 분리된 side query 실행
- 설정된 compaction model과 fallback 후보 선택
- structured state snapshot 형태의 summary prompt
- 압축 뒤 필요한 상태를 restoration attachment로 보강

사용자는 `/compress`로 LLM summary를 요청할 수 있고 `/compress-fast`로 오래된
tool output과 thinking을 규칙 기반으로 빠르게 줄일 수 있다. 두 명령을 분리한
점은 손실 종류와 비용을 사용자에게 드러낸다.

## 상태 복원

Qwen Code 설계는 최근 tail을 무조건 많이 보존하기보다 summary가 놓치기 쉬운
상태를 선택적으로 다시 붙이는 방향도 사용한다. 예를 들어 최근에 접근한 파일의
현재 내용을 restoration attachment로 제공할 수 있다.

이 방식은 코딩 작업의 실용적 연속성에는 도움이 된다. 그러나 다음 두 상태를
구별해야 한다.

- 과거 model request 당시 모델이 실제로 보았던 파일 내용
- compaction 시점에 filesystem에서 다시 읽은 현재 파일 내용

두 번째 값을 첫 번째 값처럼 replay하면 historical transcript의 의미가 바뀐다.
Qwen Code처럼 live workspace continuation을 우선하는 도구에는 합리적일 수 있지만,
Yo의 exact-replay Journal에는 별도 provenance가 필요하다.

## 실패와 운영상 특성

Side query는 main model request와 다른 streaming/gateway 경로를 거칠 수 있어
timeout이나 Provider compatibility 문제를 별도로 만든다. compaction model
fallback은 가용성을 높이지만 summary의 품질과 tokenizer, privacy boundary가
request마다 바뀔 수 있다.

Qwen Code의 상세 telemetry는 어느 단계에서 token을 줄였는지 구분하는 데 도움이
된다.

- micro-compaction으로 제거한 token
- LLM summary request usage
- summary 전후 context size
- 보존한 최근 result와 제거한 path
- 압축 실패 원인

## Yo에 대한 적용 판단

가져올 점:

- warning, compact, reject를 서로 다른 상태로 본다.
- summary 비용과 실제 context 절감량을 분리해 보고한다.
- fast deterministic compaction과 semantic LLM compaction을 사용자와 계약에서
  구분한다.
- 파일 또는 상태를 재주입한다면 historical data와 live observation의 provenance를
  표시한다.

첫 구현에서 제외할 점:

- 오래된 tool result와 private thinking의 자동 삭제
- compaction 전용 별도 model과 fallback chain
- 현재 파일 내용을 historical suffix 대신 조용히 사용하는 것
- gateway 오류 후 다른 Provider로 summary를 반복하는 것

Yo의 첫 strategy는 visible semantic prefix summary만 수행해야 한다. Qwen식
micro-compaction은 향후 `trim-old-tool-results/...`처럼 별도의 versioned strategy,
loss disclosure와 exact before/after measurement를 갖춘 후 추가하는 편이 맞다.

## 출처

- [Qwen Code chat compression service](https://github.com/QwenLM/qwen-code/blob/main/packages/core/src/services/chatCompressionService.ts)
- [Qwen Code micro-compaction implementation](https://github.com/QwenLM/qwen-code/blob/main/packages/core/src/services/microcompaction/microcompact.ts)
- [Qwen Code settings](https://github.com/QwenLM/qwen-code/blob/main/docs/users/configuration/settings.md)
- [Qwen Code commands](https://github.com/QwenLM/qwen-code/blob/main/docs/users/features/commands.md)
- [Qwen Code compaction design discussion](https://github.com/QwenLM/qwen-code/issues/4592)
