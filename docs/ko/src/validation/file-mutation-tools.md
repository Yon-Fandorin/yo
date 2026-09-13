# 파일 수정 도구 비교

2026-09-13 사용자 지정 모델 `qwen3.8-27b`와 `qwen3.8-flash`가 candidate
`06d3b4db`에서 실제 production host 비교를 완료했다. 두 모델 모두 QwenCloud
일반 API를 사용했으며, 실행 전 모델별 무료 쿼터 1,000,000토큰과
`Free quota only` 활성화를 확인했다. Responses Token Plan 키는 사용하지 않았다.
Provider cost 필드는 없었다. 무료 사용 근거는 인증된 쿼터 보호 설정이며,
usage에 비용 0이 있었다고 주장하지 않는다.

## 방법과 검사 범위

모델별 도구 형식 2개 × 합성 작업 3개 × 반복 3회로 18개 셀을 실행했다.
작업·반복별 형식 순서를 교차 배치했다. 모든 셀은 새 임시 workspace를 사용하고,
변경하면 안 되는 파일을 포함한 최종 파일의 정확한 bytes를 비교했다.

- **떨어진 위치 수정:** 같은 파일에서 retry 동작과 label을 바꾸고 무관한 본문은 보존한다.
- **새 파일:** 정확히 두 줄인 limits 파일을 만들고 README는 바꾸지 않는다.
- **비슷한 대상 구분:** `syncUser`만 바꾸고 유사한 `syncTeam` 본문은 보존한다.

기본 형식은 production `edit_file`·`write_file`을 제공했다. 실험용 구조화
`apply_patch`는 Add/Update 문법을 같은 production 호출로 변환했다. 두 형식 모두
변경하지 않은 semantic admission·registry·`LocalToolHost`를 사용했다. 이는
production 고급 도구, 여러 파일의 원자적 수정이나 managed backend·TUI 흐름을
검증한 결과가 아니다.

정확한 endpoint는
`https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions`였다.
두 형식 모두 temperature 0, 출력 한도 2,048토큰, `enable_thinking: false`, 첫
라운드 필수 도구 선택과 이후 자동 선택을 사용했다. 셀당 최대 4회, 모델당 최대
72회 요청으로 제한했다. HTTP 오류가 발생하면 matrix를 중단하도록 했으며,
redirect·자동 재시도·다른 모델·구독 fallback은 비활성화했다. 같은 제한된 셀에서
잘못된 호출을 수정하는 요청도 추가 모델 요청으로 센다.

## 관측 결과

각 행은 셀 9개의 호출·토큰 합계다. 시간은 transport·host 실행을 포함한 셀별
소요 시간의 합계이며, 한 host 환경의 작은 표본이지 지연시간 보장이 아니다.

| 모델 | 형식 | 정확한 bytes 통과 | 요청 / 도구 호출 | Production 호출 | 잘못된 호출 | 입력 / 출력 토큰 | 전체 토큰 | 인자 bytes | 시간(ms) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `qwen3.8-27b` | 기본 | 9/9 | 9 / 9 | 9 | 0 | 5,775 / 801 | 6,576 | 1,797 | 29,045 |
| `qwen3.8-27b` | 구조화 패치 | 9/9 | 10 / 10 | 9 | 1 | 5,071 / 854 | 5,925 | 2,289 | 30,746 |
| `qwen3.8-flash` | 기본 | 9/9 | 9 / 9 | 9 | 0 | 5,775 / 792 | 6,567 | 1,797 | 29,759 |
| `qwen3.8-flash` | 구조화 패치 | 9/9 | 9 / 9 | 9 | 0 | 4,497 / 763 | 5,260 | 2,057 | 26,231 |

36개 셀 모두 통과했으며 production 실행 실패나 누락된 토큰 usage는 없었다.
`27b`는 19회·12,501토큰, `flash`는 18회·11,827토큰을 사용했다. 전체 비교는
37회·24,328토큰이었다. `27b`의 잘못된 패치 1회는 marker 문법 오류로 host를
변경하지 않았으며, 해당 셀의 다음 요청에서 수정됐다.

이 matrix에서 패치 문법은 `27b`의 전체 토큰을 9.9%, `flash`는 19.9% 줄였다.
인자 bytes는 각각 27.4%, 14.5% 늘었다. 최종 파일 정확도는 개선되지 않았고,
`27b`는 호출 1회를 더 사용했다. 현재 기본 도구를 유지한다. 이 결과만으로 실험
adapter를 production에 추가하거나 기본 편집을 교체할 근거는 부족하다. 후속
고급 기능에는 별도의 승인된 계약과 더 넓은 검증이 필요하다.

두 모델 모두 QwenCloud를 사용한다. 사용자 지정 두 모델의 비교이며 세 번째
독립 Provider 표본은 아니다. 과거 가상 workspace 결과와 중단된 OpenRouter
실험은 이 합계에서 제외했다.

## 검증 식별 정보

실제 matrix는 아래 식별 정보의 candidate host를 사용했다. 원래 오프라인 symlink
oracle은 `write_file`이 link 자체를 교체하도록 잘못 기대했다. 독립 검토는 승인된
nonregular target 계약에 따라 이를 거절했다. 파일 유형의 bit 포함 검사 때문에
symlink·socket도 일반 파일로 허용됐다. 수정 host는 target capture·descriptor
admission·scratch 검증에서 전체 유형 field를 마스킹한 뒤 일반 파일 유형과
정확히 비교한다.

새 production-host 회귀 검사는 이전 실패를 재현하며 일반 파일·dangling·credential
symlink와 directory·FIFO·socket을 확인한다. 모든 시도는 정확한 `unavailable`
출력으로 실패하고 entry identity·mode·link 대상·참조 bytes를 보존하며 scratch를
남기지 않는다. 수정 bridge에서 오프라인 작업 fixture 6개와 모호한 대상·경로 이탈·
symlink entry 보존·namespace·파서·무료 admission 대조 검사가 통과했다.
Tools package는 77개 통과·1개 ignored였으며 ignored 항목은 검증으로 세지 않는다.
실제 matrix는 일반 파일만 사용했다. 원래 결과의 candidate·host identity를 유지하며
다시 서비스 요청을 보내거나 새 모델 artifact를 검증했다고 주장하지 않는다.
모든 셀의 임시 workspace를 제거했다. 실제 API 키를 Rust host에 전달하지 않았으며
이 보고서에 인증 정보·모델 원문·비공개 추론은 보존하지 않는다.

Production host 회귀 검사는 다음과 같다.

```bash
cargo test --locked -p yo-cli write_file_rejects_nonregular_entries_without_replacing_them
```

수정한 오프라인 host SHA-256은
`fe5a1a90f861e57925492829c724a9541fe81367b630472a4290b57b27712f30`이다.

원래 실제 matrix의 host SHA-256은
`160e6367da5feb60a32ae420885ee0f979433ae50239b3ff2a31cc2e86fa9a51`이다.
고정한 runner·bridge·fixture source SHA-256은 각각
`57110c6a5e3f1bb206e23bd9459d400fe37451c0fff8546ac22ee09a65dc6dcf`,
`363a46868ecd2dfce21be7baa975630bbba496b9c517fd3304325fb775d9c4cf`,
`615be4a57766bdabc25b4e490faeff788db9cd299635b4c483facc0400256781`이다.
