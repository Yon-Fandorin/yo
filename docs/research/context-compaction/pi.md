# Pi coding-agent compaction

> Status: non-authoritative research input
>
> 조사 기준일: 2026-08-29

## 시작 조건

Pi의 기본 자동 압축 조건은 다음과 같이 단순하다.

```text
contextTokens > contextWindow - reserveTokens
```

기본 `reserveTokens`는 16,384다. 모델이 다음 응답을 생성할 공간을 남기는
목적이다. 자동 압축은 기본 활성화되어 있고 설정에서 끌 수 있으며 사용자는
`/compact`로 수동 압축할 수 있다.

여기서 `contextTokens`는 항상 Provider에 보내기 직전 payload를 tokenizer로
정확히 센 값은 아니다. Pi는 최근 assistant usage가 유효하면 이를 사용하고,
그렇지 않으면 메시지 내용을 로컬에서 추정한다. 이 때문에 Provider의 hidden
prompt, tool schema 직렬화와 tokenizer 차이가 크면 실제 한계와 판단 시점이
어긋날 수 있다.

## 절단 경계와 최근 원문 보존

Pi는 최신 메시지에서 과거 방향으로 걸으며 기본 `keepRecentTokens`인 약 20,000
token을 모은 지점에 cut point를 잡는다. 오래된 구간은 summary가 되고 그 뒤의
최근 메시지는 원문으로 남는다.

유효한 cut point를 역할 경계에 맞추며 tool result 앞에서 임의로 자르지 않는다.
따라서 assistant의 tool call과 대응하는 result가 서로 다른 쪽에 놓이는 잘못된
context를 피한다.

한 turn 자체가 20K보다 길면 cut point가 turn 중간에 놓일 수 있다. Pi는 이 경우
다음 두 내용을 따로 요약한 뒤 합친다.

- 앞선 전체 history의 summary
- 보존 경계 이전에 잘린 현재 turn prefix의 summary

이 구조는 아주 긴 tool-heavy turn 하나가 최근 원문 예산을 모두 차지하는 경우를
다룬다.

## 반복 압축

첫 압축 이후 모델 입력은 대략 다음과 같다.

```text
compaction summary + messages from firstKeptEntryId onward
```

다시 압축할 때는 이전 summary를 새 summary 작성의 입력으로 사용한다. 이전에
살아남았던 메시지도 다음 압축 범위에 포함할 수 있도록 이전
`firstKeptEntryId`를 기준으로 summarized span을 재구성한다. 즉 summary만 계속
summary하는 구조보다 최근 원문이 어떻게 흡수되는지 명확하다.

기본 summary는 다음과 같은 작업 인계 정보를 구조화한다.

- 목표와 사용자 의도
- 제약과 요구사항
- 진행한 작업과 남은 작업
- 중요한 결정과 이유
- 다음 단계
- 계속 작업하는 데 필요한 핵심 context
- 읽거나 수정한 파일

summary 최대 token은 reserve budget의 일부와 모델 output 한도 중 작은 값으로
제한된다. 조사한 구현에서는 일반 compaction에 대략 `0.8 * reserveTokens` 상한을
적용했다.

## 영속화와 복구

Pi의 session JSONL에는 `CompactionEntry`가 추가된다. 핵심 필드는 다음과 같다.

- 사람이 읽을 수 있는 `summary`
- 최근 원문이 시작되는 `firstKeptEntryId`
- 교체되기 전 context의 `tokensBefore`
- 확장 기능이 추가한 details

session을 다시 열 때 원래 transcript를 물리적으로 덮어쓰지 않는다. 가장 최근
compaction entry를 찾아 summary message를 만들고, `firstKeptEntryId` 이후의
메시지를 붙여 모델 context를 복원한다. HTML export와 TUI에서도 압축 사실과
압축 전 token을 표시할 수 있다.

## 실패와 확장성

Pi는 compaction lifecycle을 extension hook으로 노출해 다른 summary 생성이나
결과 조정을 허용한다. 투명성과 확장성은 높지만, extension 결과가 어떤 역사와
결속됐는지 Yo 수준의 source Anchor나 atomic epoch graph로 검증하지는 않는다.

또한 token 추정이 실제 Provider payload보다 작으면 압축을 너무 늦게 시작할 수
있다. 이 경우 Provider overflow 이후 복구 의미는 Provider adapter와 주변
오류 처리에 영향을 받는다.

## Yo에 대한 적용 판단

Pi는 Yo 첫 구현의 가장 가까운 비교 대상이다.

가져올 점:

- visible summary와 최근 원문 suffix를 분리한다.
- tool call/result를 가르는 cut point를 허용하지 않는다.
- 반복 압축에서 이전 summary와 보존 경계를 명시적으로 입력한다.
- 압축 사실, 이전 token 수와 retained boundary를 영속화한다.
- summary에 목표, 결정, TODO와 파일 작업을 구조화한다.

Yo가 더 엄격해야 할 점:

- 문자/usage 추정이 아니라 Connector가 만든 전체 입력을 정확히 센다.
- entry ID 하나가 아니라 source Continuation Anchor와 semantic sequence, binding
  epoch를 함께 검증한다.
- summary 실패 시 partial compaction entry를 남기지 않는다.
- summary와 successor epoch open을 하나의 atomic Journal transition으로 만든다.
- private reasoning, credential과 uncommitted effect를 summary 입력에서 제외한다.

Pi의 16,384 reserve와 20K recent 값은 구현 예시이지 Yo의 계약 값이 아니다. Yo는
이미 exact input limit의 90%와 newest complete semantic group 보존을 승인했으므로
고정 수치를 추가하면 별도 의미 변경이 된다.

## 출처

- [Pi compaction documentation](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/compaction.md)
- [Pi settings documentation](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/settings.md)
- [Pi compaction implementation](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/src/core/compaction/compaction.ts)
- 로컬 교차 확인: `earendil-works/pi` commit
  `4a98f748bb11a09f5965d29f463ef7ba1851de69`
