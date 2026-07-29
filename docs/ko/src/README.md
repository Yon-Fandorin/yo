# yo 개발 문서

이 첫 번째 개발 문서는 `yo`를 변경하는 사람과 코드 에이전트를 위한
공간이다. 아직 `yo`의 설치와 사용법을 안내하는 제품 문서는 아니다.

이 문서에서 다음 내용을 확인할 수 있다.

- 변경할 크레이트나 모듈의 소유 경계를 찾는다.
- 여러 크레이트를 지나는 실행 경로를 따라간다.
- 관련된 결정론적 검사와 환경 의존적 검사를 찾는다.
- 설계 제약을 소유하는 Methexis 결정이나 계약으로 이동한다.

## 상황에 맞는 기준 문서 선택하기

| 질문 | 기준 문서 |
|---|---|
| `yo`는 무엇이며 현재 어디까지 공개되어 있는가? | 저장소 [`README.md`](https://github.com/Yon-Fandorin/yo/blob/develop/README.md) |
| 브랜치, Slice, 리뷰, 커밋은 어떻게 관리하는가? | [`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md) |
| 코드는 어디에 있고, 어떻게 실행하고 검증하는가? | 이 개발 문서 |
| 승인된 동작이나 설계 제약 중 반드시 유지할 것은 무엇인가? | Methexis KnowledgeUnits |

Methexis KnowledgeUnits는 승인된 설계 결정과 동작 계약을 소유한다.
이 개발 문서는 코드 탐색, 현재 동작에 대한 설명, 검증 방법을 소유한다.
계약이 중요할 때는 같은 내용을 또 하나의 기준으로 적지 않고 해당
KnowledgeUnit으로 연결한다.

영어 문서가 개발 문서의 canonical 원문이다. 이 한국어 문서는 같은 페이지
집합을 검토할 수 있도록 옮긴 Projection이다. 검증 과정은 기록된 영어
원문 해시가 오래된 Projection을 거부한다. 다만 번역의 의미가 정확한지는
여전히 번역 리뷰에서 확인한다. 같은 페이지의 다른 언어로 이동할 때는
화면 위쪽의 언어 전환 버튼을 사용한다.

처음 변경할 때는 다음 순서로 읽는다.

1. [아키텍처](./architecture/overview.md)에서 전체 시스템 형태를 파악한다.
2. [코드 지도](./architecture/code-map.md)에서 소유자를 선택한다. 코드
   위치보다 관찰 가능한 결과에서 출발하려면
   [변경 지점 찾기](./workflows/find-the-change.md)에서 시작한다.
3. 변경이 여러 경계를 지난다면 [실행 흐름](./architecture/runtime-flow.md)을
   따라간다.
4. [검증](./validation/)에서 개발 중에 실행할 집중 검사와
   Slice를 닫기 전에 실행할 검사를 선택한다.

## 한국어 Projection 관리하기

canonical 페이지가 바뀌면 `docs/ko/src` 아래의 같은 경로에 있는 한국어
페이지도 수정하고, 새 원문 해시를 승인하기 전에 번역의 의미가 정확한지
검토한다. 페이지 집합, link destination, heading, list, table, code fence는
서로 맞게 유지한다. 저장소 검증이 이 기계적인 경계를 확인한다.

번역 검토가 끝나면 `docs/src`에서 변경된 canonical 페이지의 해시를
계산한다.

```bash
(cd docs/src && shasum --algorithm 256 path/to/page.md)
```

출력된 값으로 `docs/ko/source.sha256`에서 해당 페이지의 한 행만 바꾼 뒤
`bash tools/validation/developer-docs.sh`를 실행한다. stale Projection
검사를 조용히 통과시키려는 목적으로 해시만 새로 만들면 안 된다.
