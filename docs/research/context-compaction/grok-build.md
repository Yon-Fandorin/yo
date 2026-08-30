# Grok Build compaction

> Status: non-authoritative research input
>
> 조사 기준일: 2026-08-30

## 시작 조건과 사용자 제어

Grok Build는 기본적으로 context 사용률 약 85%에서 자동 compaction을 수행한다.
사용자는 `/compact` 명령으로 직접 시작하고 추가 지침을 줄 수 있다. 설정에는
별도 compact model 선택과 optional two-pass compaction이 있다. Two-pass는 기본
비활성화다.

Compaction policy에는 memory flush와 전용 실행 제한 시간이 포함될 수 있다.
조사한 구현의 compaction wall-clock bound는 300초였다. 긴 summary가 main agent를
무기한 막지 않게 하지만 main request deadline과 별도의 시간 정책을 만든다.

## 대화와 실행 상태의 분리

Grok Build의 특징은 단순히 transcript를 요약 prompt에 넣는 데서 끝나지 않고
현재 작업 상태를 별도 seed로 구성한다는 점이다. compaction utility가 수집하는
대표 상태는 다음과 같다.

- 마지막 실제 사용자 query
- 편집한 path
- 실행 중인 task와 sub-agent
- 연결된 MCP server
- TODO
- project instruction
- 전체 raw transcript의 위치

이는 summary 모델이 긴 대화 속에서 현재 작업의 핵심을 스스로 다시 찾아야 하는
부담을 줄인다. 최근 메시지를 상태 seed와 중복해서 summary prompt에 넣지 않도록
compaction view를 조정하기도 한다.

## Summary 결과 처리

Summary는 continuation에 필요한 현재 상태를 구조화하도록 유도된다. 구현은
빈 문자열, 지나치게 짧은 응답이나 prompt를 되풀이한 degenerate output을
검사하고 결과를 sanitize한다. 유효하지 않은 결과는 bounded evidence를 남긴 뒤
다시 시도할 수 있다.

상세 prompt는 최종 결과를 하나의 `<summary>` block과 번호 section으로 요구한다.
사용자 의도, 기술 개념, 파일과 code, 오류와 해결, 문제 해결 상태와 모든 실제
user message가 주된 section이다. Short-prompt mode는 rigid section 대신 successor
assistant가 이어서 작업할 수 있는 자유형 summary를 요구한다. 즉 Grok Build도 한
가지 고정 format만 쓰지 않는다.

Model이 만든 본문 밖에서 runtime은 마지막 실제 user query, edited path, 실행 중
task와 sub-agent, MCP server, TODO, project instruction을 typed state로 다시 붙인다.
Summary에는 raw transcript path를 담은 `<transcript_location>` pointer도 추가할 수
있다. 이 분리는 모델이 현재 상태 값을 추측하거나 긴 목록을 되풀이하지 않게 한다.

Two-pass를 켜면 첫 pass에서 넓게 추출하고 두 번째 pass에서 정리하거나 검증하는
형태를 사용할 수 있다. 품질은 높아질 수 있지만 request 수와 latency가 늘며 두
pass 사이의 실패 상태도 정의해야 한다.

## 영속화와 원문 접근

압축된 model context와 raw session transcript를 구분한다. session log는 별도로
남아 있고 compaction checkpoint가 이후 context를 구성한다. summary에는 필요할
경우 전체 transcript 위치를 가리키는 pointer를 포함할 수 있다.

이 구조의 의미는 다음과 같다.

- 모델의 기본 입력은 작게 유지한다.
- summary가 빠뜨린 정보가 있더라도 raw transcript 자체는 삭제하지 않는다.
- tool이 허용된 환경에서는 필요한 과거 원문을 나중에 찾아볼 수 있다.
- summary loss와 durable history loss를 구분한다.

다만 transcript pointer가 실제 복구 수단이 되려면 모델이 해당 path를 읽을 수
있어야 한다. 이는 filesystem 권한, session privacy와 tool policy의 일부다.

## 실패와 관측

Grok Build는 degenerate summary를 검출하고 재시도하며 rejected output의 일부를
진단용으로 남긴다. 이 방식은 summary 품질을 높이지만 다음 비용이 있다.

- 정확히 몇 번 Provider request가 발생할지 사전에 단순하게 말하기 어렵다.
- rejected output에 private history가 다시 나타나지 않도록 bounded redaction이
  필요하다.
- 별도 compact model을 쓰면 Provider/model binding이 바뀐다.
- 300초 compaction deadline과 main request lifecycle의 관계를 정의해야 한다.

## Yo에 대한 적용 판단

가져올 점:

- narrative summary와 durable/current execution state를 구분한다.
- 마지막 사용자 의도, TODO, modified paths와 실행 중 작업을 명시적으로 보존한다.
- raw Journal을 summary로 덮어쓰지 않는다.
- compaction 결과가 너무 짧거나 구조를 잃었는지 deterministic validation을
  수행한다.
- summary가 손실 표현임을 드러내고 원본 Anchor를 유지한다.

첫 구현에서 제외할 점:

- 두 단계 summary와 degenerate-output retry
- compaction 전용 model 선택
- main request와 별도인 300초 deadline
- model이 arbitrary raw Journal path를 직접 읽는 기능
- rejected Provider output을 durable evidence로 보존하는 것

Yo는 이미 anchored Journal을 보유하므로 transcript pointer 대신 handoff record의
source Continuation Anchor와 first retained semantic sequence를 사용할 수 있다.
향후 과거 원문 조회가 필요하면 raw path를 prompt에 노출하기보다 Journal의
권한·redaction을 따르는 bounded read tool로 설계해야 한다.

## 출처

- [Grok Build repository](https://github.com/xai-org/grok-build)
- [Grok Build configuration](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/docs/user-guide/05-configuration.md)
- [Grok Build slash commands](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/docs/user-guide/04-slash-commands.md)
- [Grok Build compaction policy](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-agent/src/compaction.rs)
- [Grok Build compaction state utilities](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-chat-state/src/compaction_utils.rs)
- [Grok Build compaction prompt and request assembly](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-shell/src/session/helpers/session_compact.rs)
