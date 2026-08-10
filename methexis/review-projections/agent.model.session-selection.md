---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.model.session-selection
revision: sha256:5f5c45cfcea9cf78d74614a1b18b88665a6caa0bc922bbdc6eef32f5f67574ee
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:cf1e9b9bf64df84219e1ea269080ce3152d01d6b174b4746452e4f9ce226df5a
---
# Korean Review Projection

## Translation

# Session 모델 선택

## 계약

Interactive 및 non-interactive startup은 선택적인 `--model MODEL_REFERENCE` 하나를 받아야 합니다. 이를 생략하면 설정된 `model.startup`을 유지해야 하며 startup binding이 없으면 계속 Codex로 시작해야 합니다. 새 Session에 reference를 제공하면 `model.startup` 없이도 설정된 Yo-managed model을 선택할 수 있어야 합니다.

`MODEL_REFERENCE`에는 사용자 표기 `Model`, `Provider::Model`, `Provider:Account:Model` 세 가지가 있습니다. Resolver는 적용 가능한 configured catalog coordinate가 각 표기로 만드는 byte와 입력을 비교하고 같은 coordinate를 중복 제거한 뒤 정확히 하나가 남을 때만 성공해야 합니다. Separator 우선순위로 ModelId 해석과 qualified 해석 중 하나를 조용히 선택해서는 안 됩니다. ModelId byte는 vendor가 소유하며 `:`, `/`, `.`을 포함할 수 있습니다. Provider와 Account display name은 해석에 참여하지 않습니다.

Startup binding이나 현재 Yo-managed binding이 namespace를 제공하면 `Model`은 현재 Provider와 Account 안에서 해석합니다. Startup namespace가 없는 새 Codex-default Session에서는 configured catalog에서 exact ModelId만 찾을 수 있고 전체에서 coordinate 하나로 고유해야 합니다. `Provider::Model`은 exact ProviderId와 ModelId를 맞추고 configured Account coordinate가 정확히 하나여야 합니다. `Provider:Account:Model`은 세 ID를 모두 정확히 맞춥니다. Match가 없으면 absent, 여러 개면 ambiguous로 실패해야 합니다. 진단은 요청을 구분할 수 있는 안정적으로 정렬된 완전한 Provider, Account, Model coordinate를 반환해야 합니다.

TUI에서 값 없는 `/model`은 Yo-managed binding을 위한 Rib 스타일 grouped picker를 열어야 합니다. `/model MODEL_REFERENCE`는 startup과 같은 frontend-neutral resolver를 사용해야 합니다. Picker는 configured entry를 `Provider -> Account -> Model` 순서로 group하고 각 row를 display text나 ModelId만이 아니라 complete binding으로 식별해야 합니다. 선택을 commit하기 전에 credential, tokenizer, protocol, connector, endpoint, staleness를 계속 검증해야 합니다.

Resume한 Yo-managed exact-replay Session도 같은 reference 문법을 사용할 수 있습니다. Bare `Model`은 최신 durable Continuation Anchor의 Provider와 Account 안에 머물며 qualified form은 다른 configured coordinate를 지정하고 기존 exact-replay replacement transition을 요청할 수 있습니다. Configured startup default는 resume namespace를 대체해서는 안 됩니다. Durable Anchor가 없으면 continuation 계약에 따라 read-only로 남습니다.

이 revision은 cross-backend handoff를 허용하지 않습니다. Codex로 시작한 live Session은 model picker를 노출해서는 안 되며 Codex resume과 Yo-managed model reference의 조합은 별도 검토된 transition이 committed semantic boundary에서 exact replay를 만들고 새 backend epoch와 cache-loss boundary를 기록하며 Codex-owned input provider를 교체할 수 있을 때까지 명시적으로 실패해야 합니다.

TUI 선택은 현재 Yo Session만 변경합니다. Default-model persistence는 별도의 settings action입니다. Switch는 old binding을 usable하게 유지한 채 준비하고 active Turn 중에는 거부하며 새 binding epoch로 atomic commit해야 합니다. Preparation, replay, publication failure는 old binding을 usable하게 유지하고 partial epoch를 만들면 안 됩니다. 이전 message는 자신을 만든 exact binding attribution을 유지합니다.

## 이유

Compact option 하나는 반복적인 startup을 model-first로 유지하면서 exact catalog matching으로 Provider, Account, Model identity를 보존합니다. Contextual shorthand는 반복 좌표 입력을 줄이고 complete reference는 ambiguity에서 빠져나오는 명시적 수단이며 catalog-derived matching은 별도 escaping 문법 없이 vendor ModelId 문장부호를 보존합니다. Startup selection을 cross-backend replay와 분리하면 Codex-managed state를 이미 local exact replay로 바꿀 수 있다고 가장하지 않으면서 지금 유용한 OpenAI-compatible startup을 제공합니다.
