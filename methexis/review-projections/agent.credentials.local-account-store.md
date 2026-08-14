---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.credentials.local-account-store
revision: sha256:0ae0d11b00139c2b931f496e1c4e3033b9d4ada3def8312c738dc0acf1ece40f
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:e223c3470618d5265af3bdac4987a1f8ada5f2715435b51a60f4f11bff052266
---
# Korean Review Projection

## Translation

# Provider 범위 로컬 자격증명 저장소

## 규칙

API 자격증명은 공개 설정과 분리합니다. 첫 로컬 저장소는 선택된 Yo `config.yaml` 옆의 버전이 있는 `credentials.yaml`이며, 안정적인 ProviderId와 AccountId 순서로 이름공간을 나눕니다. 이 좌표는 비밀값만 선택합니다. 서로 다른 Provider 아래의 같은 AccountId는 서로 독립적이고, 완전히 같은 좌표가 중복되면 실패하며, 기존 ID는 계속 유효합니다.

자격증명 캡처는 유효 바인딩이 외부 자격증명을 요구할 때만 수행합니다. 파일은 no-follow 방식으로 한 번 열고 불변의 정확한 좌표를 검증하며, 자격증명이 없으면 요청 전에 실패합니다. Local Codex처럼 외부 자격증명이 필요 없는 바인딩에는 자격증명 경로가 필요하지 않습니다. 다른 계정으로 fallback하지 않습니다. 열린 handle은 현재 사용자가 소유하고 group 또는 world 권한 비트가 없는 일반 파일이어야 합니다. 읽기는 크기가 제한되고 그 handle만 사용하며, identity나 관련 metadata가 바뀌면 거절합니다.

경로가 없으면 예약된 opaque revision token `absent`와 pair가 없는 표준 빈 snapshot으로 봅니다. 저장소 lock 아래에서 `prepare`는 snapshot을 다시 읽고, pair 하나에 대한 동작을 예상 CredentialRevision과 새로 예약한 non-absent 예정 CredentialRevision에 결합합니다. 준비만으로 저장소 byte는 바뀌지 않습니다. 예정 revision은 비밀값이나 파일 byte에서 만들지 않고 독립적으로 생성하며, commit 전에 연결 orchestrator가 durable하게 기록할 수 있습니다. 준비된 mutation은 정확한 expected revision, planned revision, pair, 그리고 `add`·`replace`·`remove` 중 하나에 결합되어 다른 대상으로 바꿀 수 없습니다.

`commit`은 그 준비된 mutation만 받고, `add`나 `replace`인 경우에는 메모리에 남아 있는 secret도 받습니다. 현재 상태가 expected revision 또는 정확한 planned revision이 아니면 거절합니다. planned revision과 의도한 pair 동작이 이미 적용되어 있으면 idempotent success이고, 그 밖의 winner는 conflict입니다. 최초 생성은 경로 부재를 다시 확인하고, 같은 directory에 현재 사용자 소유의 mode `0600` 일반 임시 파일을 배타적으로 만든 뒤 완전한 버전 byte를 durable하게 기록하고, expected revision이 여전히 `absent`일 때만 atomically 게시합니다. 기존 mutation도 정확한 expected revision을 기준으로 같은 bounded complete replacement와 atomic publication을 수행합니다. 확인하지 않은 winner를 덮어쓰지 않으며, 실패 시 이 operation의 임시 파일만 제거합니다.

성공 후에는 완전한 이전 snapshot 또는 새 snapshot 하나만 남습니다. 정확히 한 pair만 바뀌고 관련 없는 pair는 byte-equivalent하게 유지되며, 이미 정확히 적용된 교체나 삭제는 idempotent합니다. 마지막 pair를 삭제하면 새 non-absent planned revision을 가진 표준 버전 빈 파일을 게시할 수 있지만, 경로가 생성된 적 없거나 현재 없는 경우를 뜻하는 예약값 `absent`로 되돌아가면 안 됩니다.

CredentialRevision은 private opaque CAS이자 복구 receipt입니다. 권한이 제한된 로컬 자격증명 snapshot, secret-safe store API, 그리고 모델 서비스 계약이 소유하는 권한 제한·redacted 연결 operation journal에만 나타날 수 있습니다. 사용자에게 보이는 partial outcome, 일반 diagnostics와 logs, Session Journal, Request Audit, binding evidence, 공개 설정에서는 제외합니다. 저장소 API는 내부적으로 prepare 또는 commit status와 정확한 expected/planned opaque revision만 노출하고 secret byte는 반환하지 않습니다. 바인딩 검증, operation locking, 공개 저장소 순서, command-local config 조합, 저장소 간 복구는 모델 서비스 계약의 책임입니다.

환경 변수, command-line secret 값, standard input, child process로 key를 공급하지 않습니다. Interactive setup은 controlling TTY의 no-echo channel로 읽습니다. Non-interactive external connect는 명시적인 `--yes` authorization과 함께 `--credential-file PATH` 하나만 받습니다. 이 path는 secret material이 아니라 locator입니다. Yo는 마지막 path를 no-follow 방식으로 정확히 한 번 열고, 그 handle이 현재 사용자 소유의 일반 파일이며 mode가 정확히 `0400` 또는 `0600`인지 요구합니다. 최초 크기가 16,386 byte를 넘으면 거절하고, overflow를 판별하기 위한 byte 하나를 더 확인하면서 파일 전체를 EOF까지 읽습니다. 캡처 길이가 16,386 byte 이하이고 read 전후의 안정적인 크기와 정확히 같을 때만 받아들입니다. 읽는 동안 identity, size, ownership, permission 또는 관련 timestamp가 바뀌면 거절하고, 마지막 LF 또는 CRLF 하나만 제거한 뒤 `ApiCredential`이 허용하는 1~16,384 byte의 유효한 UTF-8인지 요구합니다. 다른 byte를 trim하거나 마지막 symlink를 따르거나 교체된 pathname을 다시 읽거나 파일 내용을 diagnostics로 보내지 않습니다. 성공하거나 실패해도 source file을 수정하거나 삭제하지 않습니다. Secret type은 display와 debug 출력을 가리고 Connector에는 정확한 pair의 opaque secret만 전달합니다. Runtime reload, refresh, failover, keychain 연동 및 다른 non-interactive secret channel은 미룹니다. 주입되는 resolver는 지금 TenantId, tenant field, tenant UI를 추가하지 않으면서도 미래에 tenant가 소유하는 선택 경계를 유지합니다.

## 이유

mutation 전에 독립적으로 생성한 CredentialRevision을 예약하면 write-ahead 복구 기록이 비밀 byte에서 identity를 만들지 않고도 정확한 예정 winner와 관련 없는 변경을 구분할 수 있습니다. 예약된 absent revision은 의도적으로 비어 있는 기존 저장소와 아직 없는 경로를 혼동하지 않으면서 최초 설치 CAS를 닫습니다. 좁은 owner-only file channel은 key를 process argument, environment, inherited input, terminal scrollback에 두지 않으면서 agent가 만든 임시 파일과 mounted secret file을 사용할 수 있게 합니다. 마지막 line ending 외의 byte를 모두 보존하여 credential identity가 조용히 바뀌는 일을 막습니다.
