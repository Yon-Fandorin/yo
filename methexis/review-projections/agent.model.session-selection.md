---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.model.session-selection
revision: sha256:efc5edce0a42b6cd3beb423804a8a3a9ff39e65426a0516fa857574a31c80946
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:ba812fcf2156c1b08c607139674dca2b89b66849cf42988178b0025cf2a412f1
---
# Korean Review Projection

## Translation

# Session 모델 선택

## 계약

Interactive와 non-interactive startup은 explicit model override인 --model MODEL_ID를 받습니다. TUI에서 /model은 Rib 스타일 selection controller를 generic selection panel로 열고 /model MODEL_ID는 direct switch를 제공합니다. Overlay는 presentation만 소유하며 frontend-neutral controller가 catalog entries, validation, preparation, accepted effect를 소유합니다.

Picker는 usable entry를 Provider에서 Account를 거쳐 Model 순서로 group하고 각 row를 display text나 ModelId만이 아니라 complete binding으로 식별합니다. Provider, Account, Model display name은 label일 뿐입니다. 초기 catalog는 validated configured entries에서 만들고 OpenAI-compatible endpoint가 complete account-scoped model-list API를 제공한다고 가정하지 않습니다. Remote catalog discovery와 caching은 미룹니다.

/model MODEL_ID는 current Provider와 Account 안의 configured entry 정확히 하나로 resolve해야 합니다. ID가 없거나 ambiguous할 때 다른 Account나 Provider를 찾지 않으며 Account 또는 Provider 변경은 grouped picker를 사용합니다. 새 Session의 --model은 configured startup Provider와 Account에서 resolve합니다. Resumed Session에서는 최신 durable Continuation Anchor의 Provider와 Account에서 resolve하고 exact replay를 통한 replacement binding을 요청하며 startup default가 그 namespace를 덮지 않습니다. Durable Anchor가 없으면 continuation contract에 따라 read-only로 남습니다. CLI와 command 모두 absence나 ambiguity를 arbitrary selection이 아니라 explicit failure로 처리합니다.

TUI selection은 current Yo Session만 변경합니다. Default-model persistence는 ordinary settings의 separate action입니다. Switch는 current binding을 변경하지 않은 상태에서 prepare하고 fully validate한 다음 새 binding epoch로 atomic commit합니다. Active Turn 중에는 거부합니다. Preparation failure, stale selection, missing credential, unsupported protocol, connector startup failure는 old binding을 usable하게 유지하고 partial epoch를 만들지 않습니다.

Model이나 Account 변경은 새 Yo Session을 만들지 않습니다. 이전 message는 그것을 만든 exact binding attribution을 유지하고 replacement binding은 continuation contract가 허용한 exact semantic replay만 받습니다.

## 이유

Model-first UX는 backend topology를 노출하지 않으면서 commercial coding tool과 맞습니다. Exact grouped binding과 resume namespace를 고정하면 duplicate model name, mutable label, startup default가 잘못된 credential이나 history를 선택하는 것을 막습니다.
