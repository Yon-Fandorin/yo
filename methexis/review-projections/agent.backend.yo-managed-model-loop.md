---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.backend.yo-managed-model-loop
revision: sha256:932bab8eb07da78a9beb325b7e040b20b62d60c4339cf60a6dabdb14d865a997
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:ca453cc31f3a27754e7be0fda98a7c56aa0f54a015e845730e7e1707b11a23c2
---
# Korean Review Projection

## Translation

# Yo-managed 모델 및 도구 루프

## Statement

Yo-managed Agent Backend는 기존 `AgentBackend` semantic port를 구현하면서 `yo-core` 안에서 모델 루프, 도구 실행 조정, 모델에 보이는 context를 소유해야 합니다. Effective binding은 Turn 시작 전에 허용된 Model Connector 하나와 정확한 API dialect를 선택해야 합니다. Model Connector는 원격 request와 stream protocol만 소유하고 dialect를 루프가 소비하는 connector-neutral round observation으로 변환해야 합니다. `yo-cli`, frontend, connector는 agent loop owner가 될 수 없습니다. 같은 루프는 dialect에서 파생된 identity를 통해 별도로 계약된 OpenAI Responses, provider-neutral OpenAI Chat Completions, Kimi Chat Completions Connector를 허용합니다. 루프는 다른 dialect를 probe하거나 다른 connector로 fallback하거나 Provider identity에 따라 분기하면 안 됩니다.

각 허용된 Turn에서 backend는 commit된 semantic Session history와 새 user input을 선택된 API dialect로 projection해야 합니다. Text delta는 기존 message segmentation과 terminal seal 경로를 통해 `ModelWork` Activity가 되어야 합니다. 모델 function call은 wire call identity, function name, 정확히 누적된 argument byte를 보존해야 합니다. Validation이 거절한 경우에도 correlated Tool Activity가 되어야 하며 invalid JSON, schema mismatch, 알 수 없거나 중복된 identity, 사용할 수 없는 tool, argument bound 실패는 effect 없이 typed validation failure로 Activity를 끝내야 합니다. Approval, admission, dispatch 전에 validation이 성공해야 합니다. Approval과 실행은 frozen registry, admission policy, execution host 경계를 사용해야 하며 모델 서비스가 로컬 workspace tool을 직접 실행하면 안 됩니다.

Backend는 다음 모델 request에서 선택된 dialect로 대응하는 function-call output을 제출하기 전에 각 function call과 정확한 tool outcome을 기록해야 합니다. 한 response가 반환한 여러 call은 scheduler가 approval과 mutable resource lease의 독립성을 증명한 경우에만 병렬 실행할 수 있으며, 그렇지 않으면 모델 순서로 실행해야 합니다. 실행 완료 순서와 관계없이 결과는 안정적인 call 순서로 반환해야 합니다. 빠지거나 중복되거나 잘못 연계된 call 또는 result는 Turn을 실패시켜야 합니다.

루프는 모델이 최종 assistant message를 내거나, cancellation이 수락되거나, 제한된 model round 한도에 도달하거나, typed failure가 발생할 때까지 model response, local tool execution, tool-result submission을 반복합니다. Session당 active Turn 하나 제한을 유지합니다. Cancellation은 진행 중인 connector 작업을 신속히 중단하고 새 tool 실행을 막고 active Activity를 interrupted로 seal한 다음 connector와 tool cleanup을 실행해야 합니다.

Absolute model-request work deadline이 있다면 루프가 이를 소유해야 합니다. 이 deadline은 선택 사항이며 기본값은 없음이어야 합니다. Agent가 설정하면 logical model request 하나에 대해 한 번 시작하고 해당 request의 모든 제한된 connector-internal retry를 포함하며, transport byte, model output, decoded event, retry로 reset되면 안 됩니다. Tool result 뒤의 다음 model request 또는 이전 failure 뒤 별도로 admit된 request는 새 deadline을 받아야 합니다. Whole-Turn wall-clock budget은 별도의 선택적 cancellation policy이며 per-request deadline에서 추론하면 안 됩니다. 두 absolute budget이 모두 없어도 connector의 유한한 transport-progress, event-delivery, data, round-count, cancellation, cleanup bound가 비활성화되면 안 됩니다. Runtime deadline policy는 effective binding 밖에 있어야 하며 binding epoch를 열면 안 됩니다.

Provider response ID, cache handle, conversation ID는 diagnostic correlation으로 보존할 수 있지만 유일한 continuation locator가 될 수 없습니다. Yo-managed binding은 provider-native resume 대신 현재 executor가 `local_client`인 `exact_replay`를 명시해야 하며, complete effective profile은 정확한 `replay_profile`을 포함해야 합니다. 실행 가능한 continuation은 Session Journal의 최신 durable Continuation Anchor가 가리키는 replay boundary를 재구성하고 endpoint, API dialect, Provider, Account, Model, connector identity 또는 replay profile이 바뀌면 새 binding epoch를 열어야 합니다. Anchor 뒤의 commit된 mid-Turn function call, tool result, partial stream, private assistant fragment 또는 다른 suffix는 diagnostic으로만 남고 자동 continuation input이 되면 안 됩니다. Durable Anchor가 없으면 replay input을 만들지 않고 continuation 계약의 read-only fallback을 따라야 합니다. Exact replay는 message role과 order, 정확히 보이는 text, function-call과 tool-result 관계, 기록된 system 및 tool contract를 보존해야 합니다. `semantic-only/v1`은 provider-private item을 금지합니다. `kimi-private-local-plaintext/v1`은 `kimi.assistant-message/v1alpha1` schema를 선언하고 Connector의 lossless validation, visibility exclusion, byte bound, binding scope, durable encoding, exact request projection을 모두 지킬 때만 그 item을 보존할 수 있습니다. 이 item은 같은 정확한 binding identity와 replay profile에서만 Session replay authority이며, generic visible history, provider-native state, frontend observation이 될 수 없습니다. Provider cache state와 계약되지 않은 모든 private field는 제외합니다.

Partial model stream, commit되지 않은 tool result, 불확실한 request, 실패한 final response 또는 필수 private assistant item이 semantic replay delta와 함께 durable commit되지 않은 K3 round는 Continuation Anchor에 포함할 수 없습니다. 한 Yo Session 안에서 모델이 바뀌는 경우를 포함해 usage와 정확한 effective binding은 이를 생성한 model response에 귀속해야 합니다.

선택된 model catalog entry는 input-token limit, output reserve, 주입된 token counter가 사용하는 정확한 tokenizer profile을 제공해야 합니다. 선택된 Connector가 전송할 provider-private item까지 포함하여 모든 model request는 dispatch 전에 counter를 통과해야 합니다. Provider 측 implicit caching은 비용을 줄일 수 있지만 exact replay나 context admission을 바꾸면 안 됩니다. Exact replay 또는 그 private extension이 model-context bound나 replay byte bound에 더 이상 맞지 않으면 backend는 typed `context_exhausted`를 반환하고 현재 Turn을 non-resumable로 완료하며 해당 binding의 이후 Turn을 거절해야 합니다. 필수 private state를 몰래 버리거나 자르거나 redact하거나 요약하면 안 됩니다. Lossy compaction은 새 binding epoch를 여는 별도 검토된 user-visible handoff로 유지합니다.

Tool argument와 output은 Activity, 이후 model input, replay delta가 되기 전에 local tool boundary의 semantic admission gate를 통과해야 합니다. Provider-private assistant item은 선택된 Connector가 성공적으로 완료하고 correlation을 확인한 response에서만 나와야 합니다. Backend는 reasoning byte를 해석하거나 표시하지 않고 schema, epoch, bound, 정확한 visible projection을 검증합니다. Backend는 visible replay와 private replay를 하나의 semantic replay record로 함께 저장해야 하며 어느 payload도 payload가 없는 resumable-outcome correlation record에 붙이면 안 됩니다. Private byte는 user-only local Session Repository에 남고 최초 구현에서는 암호화하지 않으며 Transcript, Request trace, debug formatting, log, error, diagnostic에서 제외해야 합니다.

향후 `managed_server` executor는 같은 검증된 replay prefix를 읽고 Yo-managed Session service에서 다음 model request를 조립할 수 있습니다. 이는 두 번째 replay 의미를 정의하지 않으며 `local_client`와 같은 replay contract, ordering, bound, Anchor boundary를 사용해야 합니다. Remote repository, identity, digest, availability, retention evidence를 갖춘 별도 검토 구현 전까지 미뤄지며 현재 backend는 이를 광고하면 안 됩니다.

## Rationale

`yo-core`가 루프를 소유하면 기존 frontend-independent Session contract를 유지하면서 진정한 native backend를 제공할 수 있습니다. 명시적인 connector 경계는 Provider별 wire 동작이 실질적으로 다를 때의 전용 dialect까지 Provider 분기를 추가하지 않고 서로 다른 grammar를 약화하지 않으면서 semantic loop를 공유하게 합니다. Agent가 선택적 work budget을 소유하면 transport stall detection이나 binding identity를 약화하지 않고 의도적으로 오래 걸리는 model work를 허용할 수 있습니다. Exact semantic replay는 durable continuation을 Provider의 임시 response 보관에 결합하지 않으며 tool side effect를 Yo 자체 authority와 연계된 상태로 유지합니다. K3 reasoning을 별도의 typed non-observable replay attachment로 취급하면 이 경계를 지키면서 continuation grammar가 더 풍부하다는 이유만으로 현재 모델을 비활성화하지 않고 Yo를 적응시킬 수 있습니다.
