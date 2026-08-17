# Provider 카탈로그 관리

기존 모델 카탈로그를 갱신하거나 Kimi 같은 새 Provider를 준비할 때 이
절차를 사용한다. 이 문서는 출처 선택, 코드 소유 경계, 검증 방법을
설명한다. 승인된 동작은 계속
[model-service binding KnowledgeUnit](https://github.com/Yon-Fandorin/yo/blob/develop/methexis/knowledge/agent-runtime/agent.model.service-binding.md)이
소유한다. 이 가이드를 두 번째 카탈로그 계약으로 만들면 안 된다.

Provider별 사실은 이 페이지에 두지 않는다. 바뀔 수 있는 URL, API 형태,
profile 이름, 집중 검사 명령은 별도 런북에 둔다.

- [OpenRouter](./provider-catalogs/openrouter.md)는 인증된 runtime discovery를
  사용한다.
- [QwenCloud](./provider-catalogs/qwencloud.md)는 공식 plan allowlist에서 얻은
  release-known static registry를 사용한다.
- [Kimi](./provider-catalogs/kimi.md)는 인증된 runtime discovery, 검토된 실행
  overlay, 명시적인 로컬 private-replay 동의를 함께 사용한다.
- [새 Provider 추가](./provider-catalogs/new-provider.md)는 둘 중 하나를 고르기
  전에 출처를 분류한다. 이후 Provider는 설계가 승인될 때 자기 런북을
  갖는다.

## 먼저 출처 모델 선택하기

모델 이름 표를 복사하는 일부터 시작하지 않는다. 먼저 공식 출처가 무엇을
증명할 수 있는지 판단한다.

| 출처 형태 | 적합한 카탈로그 | 중요한 한계 |
|---|---|---|
| 인증된 API가 계정에서 사용할 수 있는 목록과 충분한 typed metadata를 반환한다 | Runtime discovery | 공식 출처라도 응답에는 bounded parsing, normalization, fail-closed availability 판단이 필요하다 |
| 공식 plan 문서가 exact allowlist를 공개하지만 신뢰할 수 있는 account-scoped inventory API는 없다 | Release-known static registry | 목록 포함은 plan을 설명할 뿐 이 credential의 현재 entitlement를 증명하지 않는다 |
| 어느 출처도 안전한 complete binding을 만들 수 없다 | `yo connect --from`으로 가져오는 explicit grouped definition | 마케팅 이름만 보고 endpoint, limit, tool, entitlement를 추론하지 않는다 |

출처 종류, 인증 지점, entitlement 의미가 달라지면 동작 설계가 바뀐 것이다.
코드를 바꾸기 전에 소유 Methexis 계약을 갱신하고 활성화한다. 이미 승인된
profile 안에서 행만 갱신하는 작업은 일반 implementation Slice로 유지할 수
있다.

## 기존 카탈로그 갱신하기

1. 이 페이지, Provider 런북, 활성 model-service binding 계약, 현재 구현을
   한 번씩 읽는다.
2. credential 없이 공식 근거를 수집한다. 정확한 URL, 확인 날짜, 관련 request
   shape, yo가 사용하는 필드를 기록한다. 바뀔 수 있는 raw response는 크기를
   제한해 로컬에 두고, 결론은 승인된 commit이나 immutable review packet에
   연결한다.
3. 공식 출처와 typed catalog를 비교한다. 각 행을 추가, 제거, 이름 변경,
   capability 변경, limit 변경, 불변으로 분류한다.
4. 근거가 있는 필드만 옮긴다. complete binding에는 normalized endpoint와
   connector, resolved model profile이 포함된다. 표시 이름은 runtime 동작의
   근거가 아니다.
5. durable state를 보존한다. 현재 카탈로그에서 행을 제거해도 이전에 저장한
   저장 binding을 다시 쓰거나 삭제하면 안 된다. 새 catalog connection에서
   그 행을 더 이상 제안하지 않을 뿐이다.
6. 승인된 UX가 요구하면 유효하지만 사용할 수 없는 inventory도 이유와 함께
   표시한다. yo가 아직 실행할 수 없다는 이유로 Provider 전용 allowlist를
   만들어 행을 조용히 숨기지 않는다.
7. Provider별 집중 검사, 변경이 닿은 공통 connection/startup 회귀 검사,
   Slice-close baseline을 실행한다. 출처 해석이나 실패 동작이 바뀌면
   fresh-context 리뷰를 받는다.

## Typed boundary를 완전하게 유지하기

Provider response나 문서의 한 행 전체를 신뢰하는 blob으로 취급하지 말고 각
필드를 따로 검토한다.

| 관심사 | 요구할 근거 |
|---|---|
| Identity | 정확한 Provider, Account, Model, catalog-profile identifier |
| Transport | Normalized HTTPS endpoint, API dialect, derived connector |
| Modalities | 명시적인 input·output modality |
| Agent use | Tool-call 지원과 reasoning presentation |
| Capacity | 정확한 의미가 확인된 양의 context·output limit |
| Runtime policy | Tokenizer, structured parameter, tool policy, replay profile |
| Availability | 사용자가 이해할 수 있는 이유를 가진 typed enabled 또는 disabled 결과 |

중복되거나 잘못된 identifier, 필수 metadata가 빠진 행, 안전하지 않은 endpoint,
과도하게 큰 inventory를 거부한다. 출처 순서 때문에 UX나 테스트가 흔들리지
않도록 normalized display name과 exact ModelId 순서로 결정론적으로 표시한다.

## 실패 경계 증명하기

최소한 테스트가 다음 실수를 구별하게 한다.

- 중복, malformed, unknown 행이나 profile이 허용된다.
- disabled 행이 사라지거나 선택 가능해진다.
- picker 취소가 secret을 읽거나 repository를 변경한다.
- dynamic response가 byte, row, redirect, time bound를 넘는다.
- static catalog가 예상하지 않은 network discovery를 수행한다.
- 현재 표에서 제거된 행 때문에 기존 저장 binding을 읽지 못하거나 그 예전
  행이 새 connect admission을 우회한다.
- 같은 coordinate의 complete-binding 필드가 달라졌는데 unchanged로 처리한다.
- upstream response나 문서 순서에 따라 표시 순서가 바뀐다.

Fixture, diagnostic, review packet, 공식 근거 캡처에 secret이나 private
credential revision을 넣지 않는다.

## 반복된 뒤에만 자동화 추가하기

출처 해석과 호환성 결정에는 판단이 필요하므로 이 가이드를 durable entry
point로 유지한다. 적어도 두 번의 Provider 갱신에서 같은 안전한 기계 작업이
반복된 뒤에만 repository skill을 추출한다. Skill은 bounded public source를
가져오고, candidate table을 normalize하고, field-level diff를 만들고, 문서화된
검사를 실행할 수 있다. 출처 authority를 결정하거나, 계약을 조용히 수정하거나,
지원되지 않은 필드를 추론하거나, secret을 노출하거나, 리뷰 없이 카탈로그를
게시하면 안 된다.
