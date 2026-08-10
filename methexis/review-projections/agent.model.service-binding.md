---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.model.service-binding
revision: sha256:fa8c6e1864e2fead43c0e751ce0d932e1e5f5764d94ab65fa54c28bc0ec80942
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:a1e251059ca1188dae64143e65461db5fb2bfa06eafeecdad8e5b1d9671ac498
---
# Korean Review Projection

## Translation

# 모델 서비스 바인딩과 관리형 연결

## 규칙

라우팅은 typed stable ProviderId, AccountId, ModelId를 사용합니다. 새 관리형 ProviderId로 `host`를 사용할 수 없습니다. 다만 기존 수동 또는 durable `host` 좌표는 안정적인 자격증명, attribution, continuation identity를 가진 qualified ModelTarget으로 유지합니다. Provider는 서비스 그룹, Account는 자격증명 범위, Model은 wire 모델 이름입니다. 외부 key는 `provider`, `account`, `model`입니다. 정확한 Responses 또는 Chat Completions dialect는 닫힌 registry를 통해 Connector 하나로 해석하며 probing과 fallback은 금지합니다.

이 unit은 `yo.binding-profile/v1`을 소유합니다. 필드는 다음 순서로 정확히 존재합니다: normalized endpoint, api_dialect, resolved connector_id, tokenizer_profile, context_limit, output_limit, reasoning_parameters, optional_request_parameters, tool_capability_policy, verification_profile. 구조화 parameter는 RFC 8785 canonical JSON을 사용하고 나머지는 버전이 있는 normalized UTF-8 또는 unsigned-decimal encoding을 사용합니다. Canonical byte는 ASCII domain `yo.binding-profile/v1`, NUL, 그리고 위 순서대로 각 field name과 value의 byte length를 unsigned big-endian 64-bit로 쓴 뒤 그 byte를 붙인 값입니다. BindingProfileDigest는 이 byte의 SHA-256에 소문자 `sha256:`를 붙인 값입니다. 알 수 없거나 중복된 v1 field는 실패합니다. Profile schema version 변경은 검토된 migration이 필요하고 다른 digest와 binding epoch를 만들며, 기존 durable profile은 기록된 version으로 해석하거나 명시적으로 실패해야 합니다.

완전한 binding identity는 Provider, Account, Model, normalized endpoint, api_dialect, connector_id, tokenizer_profile, profile schema, profile digest입니다. 어느 identity든 바뀌면 새 epoch를 엽니다. Durable attribution은 이 identity를 사용하고 display name과 secret은 제외합니다.

Operator `config.yaml`은 read-only이며 ConnectionRepository가 다시 쓰거나 pin하지 않습니다. 모든 startup과 command는 선택된 파일을 no-follow 방식으로 한 번 열고, 크기가 1,048,576 byte 이하인 일반 파일인지 확인하고, 그 handle에서 bounded read를 수행하며, 캡처 중 identity·size·관련 metadata가 바뀌면 거절하여 새로운 command-local ConfigSnapshot을 만듭니다. 파일이 없으면 일반적인 빈 수동 snapshot입니다. 캡처한 정확한 byte와 소문자 `sha256:` digest는 그 invocation만 식별하며 다음 invocation은 항상 다시 읽습니다.

ConfigSnapshot entry와 관리형 entry는 Provider, Account, Model, endpoint, dialect, connector, tokenizer, profile schema, profile digest의 정확한 identity로 조합합니다. 완전한 identity가 같으면 두 provenance를 모두 유지한 채 하나로 합칩니다. 표시에는 수동 display를 우선하고 없으면 관리형 display를 사용합니다. 같은 Provider·Account·Model 좌표에서 identity가 다르면 `BindingConflict`이며 어느 entry도 routing하지 않습니다. 진단은 두 source와 비밀이 아닌 identity 차이를 보여줍니다. 사용자는 `config.yaml`을 수정하거나 `yo disconnect`로 관리형 바인딩을 제거해 해결합니다. 최초 scope에는 `reconcile` command가 없고 한 source를 조용히 선택하지 않습니다.

Write-ahead intent를 게시하거나 관리 저장소 mutation을 commit하기 전에 command는 `config.yaml`을 다시 캡처하여 같은 command-local digest와 file identity인지 확인합니다. 다르면 credential 또는 public commit 전에 `ConfigChangedRetry`로 중단합니다. 이 guard 뒤에 수동 편집이 경쟁하면 이미 캡처한 in-flight plan을 소급해서 바꾸지 않습니다. Operation은 준비된 정확한 관리 byte를 기준으로 완료하거나 복구하고, 다음 invocation이 새 파일을 다시 읽어 조합하거나 `BindingConflict`를 보고합니다. 수동 설정은 관리 operation lock 바깥이므로 임의 editor write를 serialize한다고 주장하지 않습니다.

처음에는 `connections.yaml`인 ConnectionRepository가 관리형 binding과 account, selection 소유 preference를 관리합니다. Byte를 바꾸지 않고 prospective bounded snapshot을 준비하고 독립적으로 생성한 planned ConnectionRevision을 예약하며, 정확한 expected revision, planned revision, byte, selection transition을 하나의 불변 public mutation으로 결합합니다. Public CAS는 expected revision 또는 정확한 planned revision만 허용합니다. Planned revision과 정확한 의도 byte가 이미 보이면 idempotent success이고 다른 winner는 conflict입니다.

자격증명을 추가하거나 교체하는 `yo connect`는 복구 가능한 serialized operation 하나입니다. Operation lock 아래에서 정확한 CredentialRevision, ConnectionRevision, ConfigSnapshot digest, 기존 effective binding, prospective managed snapshot, selection transition을 캡처합니다. 새로 공개된 secret은 opaque in-memory CandidateSecret으로 유지하고, public commit 전에 계속 유효할 수 있는 모든 기존 binding과 정확한 Provider·Account pair의 모든 prospective retained binding에 대한 bounded no-tool verification request마다 이 CandidateSecret만 주입합니다. 현재 저장된 secret으로 해석하거나 fallback하면 안 됩니다. In-memory 완료 evidence는 CandidateSecret handle, 검증한 complete identity set, 준비된 정확한 credential `add` 또는 `replace`를 결합합니다. 이 complete evidence와 마지막 ConfigSnapshot guard가 모두 있어야 mutation할 수 있습니다. 필요한 기존 identity가 CandidateSecret을 거절하면 교체는 실패하고 해당 binding을 제거하거나 수정한 뒤 다시 시도하도록 안내합니다.

첫 저장소 commit 전에 orchestrator는 bounded하고 현재 사용자 소유이며 mode `0600`, no-follow인 operation journal을 durable하게 게시합니다. Journal에는 닫힌 operation kind `connect_credential_change` 또는 `disconnect`, operation identity, ConfigSnapshot digest, 정확한 expected/planned ConnectionRevision, prospective non-secret public byte 또는 durable content-addressed artifact, profile digest, phase, 닫힌 credential action 하나를 기록합니다. `add`, `replace`, `remove`는 정확한 expected/planned CredentialRevision과 exact pair intent를 기록하고, `preserve`는 expected CredentialRevision만 기록합니다. Secret, CandidateSecret identity, verification payload는 기록하지 않습니다. Journal 교체는 durable하고 atomic하며 phase `intent`가 durable하기 전에는 저장소 commit을 허용하지 않습니다.

복구는 같은 operation lock 아래에서 journal의 operation kind와 credential action을 사용합니다. `connect_credential_change`는 action이 정확히 `add` 또는 `replace`여야 합니다. Credential CAS로 exact prepared pair intent를 먼저 commit하고 phase `credential_committed`를 durable하게 기록한 뒤 public CAS, durable `public_committed`, `complete` 순으로 진행합니다. Exact state table은 다음과 같습니다. Expected credential과 expected public이면 commit되지 않은 intent를 abandon하고 retry에서 secret을 다시 받습니다. Planned credential과 expected public이면 prepared public mutation을 commit합니다. Planned credential과 planned exact public byte이면 완료합니다. 나머지는 conflict입니다. `disconnect`는 public CAS를 먼저 commit하고 phase `public_committed`를 durable하게 기록합니다. Action이 `remove`이면 expected credential과 planned exact public 상태에서 prepared credential removal을 commit하고 `credential_removed`, `complete`로 진행하며, planned credential과 planned exact public이면 완료합니다. Action이 `preserve`이면 expected credential과 planned exact public에서 credential mutation 없이 완료합니다. Expected credential과 expected public이면 어느 disconnect intent든 abandon하고, planned credential과 expected public 및 나열하지 않은 상태는 conflict입니다. Journal phase는 저장소 상태보다 뒤처질 수 있고 exact table 비교 후에만 전진할 수 있습니다. 저장소에 없는 commit을 phase가 주장하면 abandon이 아니라 conflict입니다. 복구는 임의 revision 변경에서 ownership을 추론하거나 secret을 복원하거나 revision을 발명하거나 관련 없는 상태를 덮어쓰지 않습니다. Exact retry는 idempotent합니다.

ConnectionRepository 경로가 없으면 revision token `absent`와 binding 및 preference가 없는 표준 빈 managed snapshot입니다. 최초 writer는 같은 directory에 현재 사용자 소유이고 no-follow이며 mode `0600`인 임시 파일을 배타적으로 만들고 prepared snapshot을 durable하게 기록한 뒤 expected repository revision이 계속 absent일 때만 atomically rename합니다. Concurrent winner가 있으면 다시 읽어 exact intent reuse 또는 conflict로 처리합니다. 기존 write는 bounded regular snapshot, prepared opaque revision CAS, durable atomic replace, old-or-new recovery, unrelated preservation을 사용합니다. 주입된 repository도 같은 의미를 제공해야 합니다.

Known profile은 모든 profile field를 제공합니다. OpenRouter와 QwenCloud는 provider-neutral configured Provider입니다. Snapshot schema와 digest는 명시적이고 profile drift는 조용히 바뀌지 않으며 custom profile은 advanced path로 둡니다.

외부 서비스용 interactive `yo connect`는 Provider가 생략되면 묻고 Model 하나만 선택합니다. 모든 모델이나 profile default를 추론하지 않으며 invocation 하나는 binding 하나만 바꿉니다. 캡처한 effective Provider에 수동·관리 account가 모두 0개일 때만 AccountId를 `default`로 정합니다. 그 밖에는 기존 account를 interactive하게 고르거나 새 ID를 명시해야 하고 새 account 추가에는 `--account`가 필요합니다. 같은 exact pair는 secret을 교체하거나 추가합니다. 검증은 정확한 binding union과 사용량을 공개하고 한 번 확인하며 retry하지 않습니다. 성공은 인증, 정확한 entitlement, endpoint와 dialect 수용, 유한한 경계 안의 semantic terminal result를 증명합니다. 실패 종류는 구분합니다. Session-selection 계약이 first-success preference transition을 제공하며 같은 public CAS가 binding과 함께 게시합니다.

Interactive disconnect는 관리형 target 하나를 묻거나 추론합니다. Non-interactive disconnect는 정확한 Provider와 `--account`, `--yes`가 필요하고, `--yes`는 캡처한 plan과 revision만 승인합니다. 캡처한 ConfigSnapshot과 prospective managed snapshot에서 target Provider·Account pair의 post-public effective binding set을 계산합니다. 남는 수동 또는 관리형 외부 binding이 그 pair를 계속 요구하면 credential action은 `preserve`이고, post-public dependent가 없을 때만 prepared `remove`를 허용합니다. 따라서 같은 identity의 수동·관리 binding이 합쳐진 상태에서 관리 provenance만 제거하면 수동 binding과 credential은 계속 사용할 수 있습니다. Preview는 영향받는 binding, preference transition, resume risk, 계산된 credential action을 보여줍니다. 수동 전용 entry는 Yo가 제거할 수 없고 `config.yaml`을 수정하도록 안내합니다. Public removal과 preference transition을 credential removal보다 먼저 commit하며 public-first recovery table이 모든 exact crash state를 이어가거나 완료로 인식합니다. Durable history는 유지합니다.

하나의 operation lock이 connect와 disconnect를 serialize합니다. CredentialRevision은 credential store와 권한 제한 operation journal 안에서만 private하게 유지합니다. 사용자에게 보이는 typed conflict와 partial outcome은 operation kind, phase, 계산된 preserve/remove action, expected 또는 committed ConnectionRevision과 안전한 retry 안내만 표시하고 CredentialRevision이나 secret은 노출하지 않습니다. Retry는 idempotent하며 관련 없는 data를 복원하거나 삭제하지 않습니다. Tenant 선택과 UI, rotation policy, failover는 미루되, 주입 가능한 경계로 미래 caller-owned tenant scope를 보존하고 지금 TenantId를 추가하지 않습니다.

## 이유

닫힌 canonical profile은 equality, epoch 변경, replay attribution을 이식 가능하게 합니다. Command-local config reload는 일반적인 상용 도구 동작과 맞고, exact identity conflict는 두 source가 같은 좌표를 서로 다르게 routing하는 것을 막습니다. CandidateSecret 전용 검증은 credential-first 추가 또는 교체 중 기존 binding을 보호하고, operation별 복구표는 수동 또는 관리 provenance가 계속 요구하는 credential을 삭제하지 않으면서 public-first disconnect를 보존합니다.
