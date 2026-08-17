# QwenCloud 카탈로그 관리

QwenCloud는 release-known static-registry 사례다. Alibaba Cloud가 정확한 plan
allowlist와 plan별 endpoint를 공개한다. 등록은 구조적 admission만 수행하며, 일반 모델 사용이
나중에 제공된 credential로 선택한 행을 실제 사용할 수 있는지 확인한다.

## 공식 출처

해당 profile에 맞는 Alibaba Cloud 공식 페이지를 사용한다.

- [Coding Plan](https://www.alibabacloud.com/help/en/model-studio/coding-plan)에서
  exact model allowlist와 Coding Plan endpoint를 확인한다.
- [Token Plan (Team Edition)](https://www.alibabacloud.com/help/en/model-studio/token-plan-overview)에서
  exact model·capability 표를 확인한다.
- Endpoint, region, protocol, key type을 확인해야 하면 해당 quick-start
  페이지를 함께 사용한다.

가까운 이름을 보고 model version을 추론하지 않는다. 공식 plan 목록에 있다는
사실을 특정 계정에 활성 seat, quota, entitlement가 있다는 증거로 사용하지
않는다.

## 코드 소유 경계

Static profile definition, endpoint, row, typed capability, deterministic
ordering은
[`qwencloud_catalog.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-core/src/model_service/qwencloud_catalog.rs)가
소유한다. Configuration은
[`config.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/config.rs)에서
profile을 non-routable seed로 해석한다. Shared selection과 recoverable connection
transaction은
[`external.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection/external.rs)와
[`picker.rs`](https://github.com/Yon-Fandorin/yo/blob/develop/crates/yo-cli/src/connection/input/picker.rs)에
있다.

## 갱신 절차

1. 기존 catalog profile 하나를 선택하고 정확한 공식 allowlist와 endpoint를
   확인한다. Plan이나 region의 의미가 그 profile과 더 이상 맞지 않으면 예전
   profile을 조용히 재해석하지 말고 SOT-first 작업으로 새 versioned profile을
   정의한다.
2. ModelId, modality, tool support, reasoning presentation, context limit,
   output limit, endpoint, dialect의 field-level old/new 표를 만든다. 공식 근거가
   없으면 다른 vendor 페이지나 model-family 추정값으로 채우지 말고 미확인으로
   표시한다.
3. 해당 `CatalogDefinition`과 `CatalogRow` data 또는 공식 사실을 올바르게
   표현하는 가장 작은 helper만 수정한다. yo에 필요한 runtime interface가
   없으면 유효한 image-only 행이나 다른 미지원 행을 표시하되 disabled 상태로
   유지한다.
4. 바뀐 profile의 exact row/order assertion을 추가한다. Duplicate·unknown
   profile, secret 입력 전 disabled-row 거부, secret·mutation 전 picker 취소,
   exact three-part selection을 테스트한다.
5. Stale-managed-row 회귀를 유지한다. 현재 registry 밖의 이전 저장 행은
   startup/recovery에서 계속 사용할 수 있지만 새 catalog candidate가 될 수는
   없다.

집중 검사:

```bash
cargo test --locked -p yo-core qwencloud_catalog
cargo test --locked -p yo-cli qwencloud_catalog
cargo test --locked -p yo-cli connection::input::picker
```

이 갱신 경로는 QwenCloud plan을 열거하는 network request를 의도적으로 수행하지
않는다. 공식 authenticated account inventory를 authority로 사용하려면 static
table을 임시로 확장하지 말고 새로운 discovery 설계로 취급한다.
