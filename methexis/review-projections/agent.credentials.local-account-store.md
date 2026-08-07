---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.credentials.local-account-store
revision: sha256:dd29e4e700992556bcd6d19075e4aaa73efa30c6e80905396557a88bfd35a147
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:f386ed5daf455fe05eefca050dfaa6425550406611f7d0e526442e61c7c67e4f
---
# Korean Review Projection

## Translation

# Local account credential store

## 계약

API credential은 일반 Yo settings와 분리해 저장합니다. 첫 구현은 선택된 Yo config.yaml 옆의 전용 credentials.yaml을 읽습니다. Versioned format은 stable AccountId를 secret material에 연결하며 Provider나 Model routing policy를 중복하지 않습니다. File은 no-follow semantics로 한 번만 엽니다. 그 exact opened handle은 regular file이어야 하고 다른 모든 object type은 거부합니다. 같은 handle에서 current-user ownership과 group 또는 world permission bit 부재를 읽기 전에 검사합니다. 같은 handle에서 size-bounded read를 수행하고 capture 도중 identity나 relevant metadata가 바뀌면 거부합니다. Path pre-check나 두 번째 path-based open은 이 계약을 충족하지 않습니다.

Environment variable은 API key source가 될 수 없습니다. Process는 startup 중 credential file을 한 번 읽고 검증해 immutable in-memory CredentialStore로 유지합니다. Runtime reload, refresh, account rotation, failover, interactive login, OS keychain 연동은 미룹니다. 선택한 Account나 credential이 없으면 model request 전에 실패하며 다른 Account로 fallback하지 않습니다.

일반 설정과 UI는 Account ID와 display name을 노출할 수 있습니다. Connector는 exact selected Account에 대해 resolve된 opaque secret만 받습니다. Secret type의 Debug와 display는 redaction해야 합니다. API key는 diagnostic, log, Session Journal, Request Audit, model binding evidence, command-line argument, child-process environment에 들어가면 안 됩니다.

Credential resolver는 Model Connector가 file을 직접 여는 방식이 아니라 startup assembly에 주입합니다. 미래의 tenant-aware caller는 이 boundary 전에 AccountId를 고를 수 있지만 첫 구현은 tenant state를 persist하거나 display하지 않습니다.

## 이유

Permission-restricted 별도 regular file은 장기 secret을 shell environment나 공유 가능한 settings에 넣지 않게 합니다. 한 file handle에 open, type 및 permission validation, bounded capture를 묶어 path replacement race와 special-file admission을 막습니다. Startup-only resolution은 protocol code에서 file access와 secret ownership을 분리하면서 미래 credential store를 위한 좁은 seam을 남깁니다.
