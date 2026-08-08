---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.credentials.local-account-store
revision: sha256:c2b31051123e7f06dade53fbe24f665468445a19385898324c75f692a3bee45e
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:046e554249b018593aa5da74eeece1bc797aeea4074bfca979b58a304780a24a
---
# Korean Review Projection

## Translation

# Provider 범위 로컬 credential 저장소

## 규칙

API credential은 일반 Yo 설정과 분리해 저장해야 합니다. 첫 구현은 선택된 Yo `config.yaml` 옆의 전용 `credentials.yaml`을 읽습니다. 버전이 있는 파일 구조는 secret을 먼저 안정적인 `ProviderId`, 다음으로 안정적인 `AccountId` 아래에 둡니다. 이 좌표는 credential만 선택하며 endpoint, Model, connector, API dialect 또는 표시 이름의 routing 정책을 중복 소유하지 않습니다. 서로 다른 Provider는 같은 `AccountId`를 가질 수 있고 독립적으로 해석되어야 하며, 완전히 같은 Provider와 Account 조합이 중복되면 거절해야 합니다.

파일은 no-follow 방식으로 한 번 열어야 합니다. 실제로 열린 handle이 일반 파일인지 확인하고 다른 객체 유형은 모두 거절합니다. 같은 handle에서 현재 사용자 소유권과 group/world 권한 비트 부재를 읽기 전에 확인합니다. 읽기 크기는 제한하며 같은 handle만 사용하고, 캡처 중 identity나 관련 metadata가 바뀌면 거절합니다. 경로 사전 검사나 두 번째 path 기반 open으로 이를 대신할 수 없습니다.

환경 변수는 API key 출처가 될 수 없습니다. 프로세스는 시작 시 credential 파일을 한 번 읽고 검증한 뒤 정확한 Provider와 Account 조합을 key로 하는 불변 `CredentialStore`를 유지합니다. 시작 assembly는 선택된 effective model binding에서 그 조합을 해석합니다. 정확한 조합이나 credential이 없으면 model 요청 전에 실패하며 다른 Account나 Provider로 fallback하지 않습니다. runtime reload, refresh, account rotation, failover, interactive login, OS keychain 연동은 미룹니다.

일반 설정과 UI는 Provider/Account ID와 표시 이름을 보여줄 수 있습니다. Connector에는 정확히 선택된 조합의 opaque secret만 전달합니다. Secret type의 `Debug`와 display 출력은 반드시 가려야 합니다. API key는 diagnostics, logs, Session Journal, Request Audit, model binding evidence, command-line arguments, child-process environments에 들어가면 안 됩니다.

Credential resolver는 Model Connector가 직접 열지 않고 startup assembly에 주입합니다. 추후 tenant-aware caller는 자신의 tenant scope 안에서 Provider와 Account 조합을 선택할 수 있지만, 첫 구현에는 `TenantId`, tenant field, tenant UI를 추가하지 않습니다.

## 이유

Provider 범위 Account 좌표는 `default` 같은 흔한 로컬 Account ID가 다른 Provider의 secret을 선택하는 일을 막습니다. 권한이 제한된 별도 파일은 장기 shell secret을 피하고, 시작 시 한 번 주입하는 해석 방식은 추후 tenant 소유 또는 대체 credential 저장소를 위한 좁은 확장 지점을 유지합니다.
