---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.model.session-selection
revision: sha256:745454ff373a9c4f29a1add66e5146dcdfe5c7d76ef3c1c900a636501b8a522b
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:3e028f10370257568651725ac3c6c67b28a6bfeec9a728118ffb1fc1a7a61e0e
---
# Korean Review Projection

## Translation

# Startup target과 Session 모델 선택

## 규칙

Startup target은 HostTarget 또는 ModelTarget입니다. 정확한 `host:codex`가 첫 HostTarget이며 화면에는 Local Codex로 표시합니다. 새 관리형 ProviderId로 `host`를 사용할 수 없습니다. 다만 기존 수동 또는 durable `host` 좌표는 안정적인 credential, attribution, continuation identity를 가진 qualified ModelTarget으로 남고 HostTarget에 가려지지 않습니다. Interactive 및 non-interactive startup은 선택적인 `--model TARGET_REFERENCE` 하나를 받습니다.

ModelTarget 표기는 `Model`, `Provider::Model`, `Provider:Account:Model`입니다. Provider와 Account의 canonical percent encoding은 `%`를 `%25`, `:`를 `%3A`로 바꿉니다. 소문자 escape, 불필요한 escape, malformed escape, non-UTF-8 escape는 실패합니다. Model suffix byte는 vendor가 소유합니다. Resolver는 canonical catalog 표기와 비교하고 같은 좌표를 중복 제거한 뒤 정확히 하나일 때만 성공합니다. Bare exact `host:codex`는 HostTarget이며 같은 byte의 ModelId는 qualified 표기가 필요합니다. Display name은 routing에 쓰지 않습니다.

Bare Model은 현재 Provider와 Account namespace가 있으면 그 안에서 해석하고, 없으면 전체에서 exact하게 하나인 경우만 사용합니다. `Provider::Model`은 Account가 정확히 하나여야 합니다. Full spelling은 앞의 두 segment를 decode하여 완전한 좌표를 지정합니다. 결과가 0개면 absent, 여러 개면 ambiguous이며 진단은 안정적으로 정렬된 canonical complete coordinate를 반환합니다.

Startup은 선택 가능한 source 네 개를 한 번씩 캡처합니다. Invocation layer는 선택적인 parsed `--model`, stored layer는 캡처한 ConnectionRepository snapshot의 세 상태 preference, injected PolicySnapshot은 admission rule과 `allow_user_override`, optional enforced target, optional policy-default target, operator layer는 command-local read-only `config.yaml`의 optional `model.startup`입니다. 각 layer의 부재는 parse failure가 아니라 명시적 값입니다. PolicySnapshot은 정확히 두 형태만 허용합니다. Overridable 형태는 `allow_user_override=true`, enforced target 없음, optional policy-default target입니다. Enforced 형태는 `allow_user_override=false`, enforced target 정확히 하나, policy-default target 없음입니다. 나머지 조합은 malformed policy입니다. 최초 제공 policy는 policy default가 없는 overridable 형태이며 Local Codex와 구조적으로 유효한 configured ModelTarget을 허용합니다.

필요한 source의 캡처나 구조 decode 실패, stale repository revision, 같은 좌표의 unequal manual/managed identity, malformed policy는 fatal입니다. 상위 target이 있어도 숨기지 않습니다. Enforced 형태는 enforced target을 선택합니다. 다른 invocation target은 fatal policy conflict이고 stored/operator target은 선택되지 않은 provenance로만 남습니다. Overridable 형태는 invocation, stored preference, policy default, operator `model.startup` 순서로 처음 존재하는 값을 고릅니다. Implicit target은 없으며 `host:codex`를 조용히 넣지 않습니다. 선택 가능한 source가 모두 없으면 interactive startup은 Yo Session이나 backend epoch를 만들기 전에 setup으로 들어가고, non-interactive startup은 `StartupTargetRequired`로 실패하면서 정확한 `yo connect`와 `--model host:codex` 안내를 보여줍니다. Target을 선택한 뒤에는 missing, stale, unavailable, unsupported, policy-denied 상태가 fallback 없이 fatal입니다.

Stored user preference는 이 unit만 소유하며 unset, HostTarget, ModelTarget 중 하나입니다. `yo default TARGET`은 허용된 선택을 저장하고 `--unset`은 지웁니다. Policy 또는 operator target도 없다면 삭제 후 다음 startup이 setup으로 들어갈 수 있습니다. Interactive picker는 inherited, Local Codex, complete configured model을 보여주고 non-interactive value-less command는 실패합니다.

Target 없는 `yo connect`는 Session 생성 전에 onboarding을 엽니다. Local Codex와 정확히 configured된 외부 모델 또는 새로 입력할 외부 모델을 제시하되 어느 것도 implicit default로 취급하지 않습니다. Local Codex를 선택하면 local Codex backend와 stable host identity가 사용 가능한지 검증한 뒤 HostTarget preference mutation을 준비합니다. 외부 모델을 선택하면 service-binding 계약의 credential, endpoint, dialect, entitlement, semantic terminal 검증을 마치고 ModelTarget과 managed binding mutation을 준비합니다. Non-interactive connect는 exact target 하나가 필요합니다.

캡처한 stored preference가 unset인 상태에서 처음 성공적으로 검증된 `yo connect`는 그 exact HostTarget 또는 ModelTarget을 성공 outcome과 같은 ConnectionRepository CAS로 preference에 기록합니다. 실패하거나 취소한 시도는 preference를 기록하지 않습니다. 이후 성공한 연결은 기존 preference를 유지하며 변경하려면 `yo default` 또는 명시적인 default-selection UI를 사용합니다. 동시에 일어난 첫 연결들은 같은 public revision을 두고 경쟁합니다. Exact CAS 하나만 이기고 loser는 winner의 preference를 다시 읽은 뒤 암묵적으로 교체하면 안 됩니다.

Disconnect 전에 selection은 prospective transition 하나를 계산합니다. Exact explicit ModelTarget preference가 제거되면 같은 public CAS로 지우고, 아니면 유지합니다. Preview는 이전 값, transition, effective lower target 또는 setup-required outcome을 보여줍니다. Model 제거로 HostTarget을 지우지 않습니다.

`yo model`과 `/model`은 ModelTarget만 다룹니다. Preparation은 policy, credential, tokenizer, protocol, connector, endpoint, profile digest, staleness를 검증합니다. Live TUI switch는 기존 binding을 사용할 수 있는 동안 준비하고 active Turn 중에는 거절하며, old epoch를 닫고 new epoch를 atomically 엽니다. Preparation, replay, publication이 실패하면 기존 binding을 계속 사용할 수 있어야 합니다.

Resume은 override를 보기 전에 newest durable Continuation Anchor를 선택하며 stored preference, policy default, operator `model.startup`은 사용하지 않습니다. Explicit override가 없으면 current policy와 exact credential availability 아래에서 Anchor binding을 사용합니다. Policy denial이나 missing credential이면 denial 또는 reconnect 안내와 함께 history를 read-only로 열고 fallback, Anchor mutation, epoch를 만들지 않습니다. 같은 Codex backend를 가리키는 HostTarget override는 same-binding confirmation이며 Anchor binding과 정확히 같은 override는 replacement epoch를 만들지 않습니다.

다른 binding을 지정하는 explicit override는 startup substitution이 아닙니다. Replacement에는 target의 exact semantic replay 지원과 함께 continuation-lineage 계약에 따른 admissible exact replay chain을 증명하는 source-Anchor evidence, 또는 같은 semantic boundary와 replay-content/contract digest를 성립시키는 별도 검토된 provider export가 모두 필요합니다. Backend-managed-state locator나 target capability만으로는 replay evidence가 되지 않으며 Transcript나 Request Audit data로 합성할 수 없습니다. Source evidence가 없으면 저장된 Session은 read-only로 남고 exact-replay-unavailable 안내와 함께 override가 실패합니다. 별도로 승인된 lossy transition만 제안할 수 있습니다. 이 규칙은 Codex뿐 아니라 모든 backend-managed 또는 local source에 적용합니다.

조건을 만족하면 Session continuation-lineage 계약이 소유하는 replacement-binding transition이 source epoch와 Anchor, 완전히 commit된 semantic boundary, target complete binding identity, replay executor, replay-content/contract digest, 알려진 cache-loss boundary, new epoch identity를 결합합니다. Replay preparation은 old epoch와 Anchor가 바뀌지 않은 동안 완료해야 합니다. 하나의 atomic durable Journal transition이 source epoch를 닫고 replacement epoch를 열며 Continuation lineage를 게시합니다. 실패하면 recorded strategy가 계속 동작하는 경우 original Anchor와 epoch를 사용할 수 있게 두고, 그렇지 않으면 saved Session을 read-only로 엽니다. Partial replacement는 게시하지 않습니다.

Backend-managed-state binding은 recorded locator와 검증된 backend identity를 통해서만 reconnect합니다. 다른 binding은 그 locator를 재사용할 수 없습니다. Cross-backend handoff는 미룹니다. Codex live Session에는 ModelTarget picker가 없고, incompatible Codex resume 또는 live host switch는 별도 검토된 exact-replay export나 explicit lossy transition이 backend-owned input provider를 교체하고 epoch, semantic boundary, cache/context loss를 기록할 수 있을 때까지 실패합니다.

## 이유

Implicit target을 두지 않으면 설정이 빠진 상태에서 Local Codex로 조용히 작업을 시작하지 않고 setup 필요성을 드러냅니다. 처음 성공적으로 검증된 선택만 저장하면 이후 startup의 default가 예측 가능하고, 뒤이은 `connect`가 이를 갑자기 교체하지 않습니다. 양쪽 replay evidence를 요구하면 backend-managed source가 내보낸 적 없는 prefix를 target capability만으로 발명하는 일을 막습니다.
