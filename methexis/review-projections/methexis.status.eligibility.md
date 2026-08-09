---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.status.eligibility
revision: sha256:8f48dc14abb9438264a1e2b26c5da42ed359a25b88f0072787dd4e5efd7b848e
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:7cc7dc3cf91984de6961a4c0768f6703063a140c204f7b7edae01778c2652403
---
# Korean Review Projection

## Translation

# 파생 최종 eligibility 상태

## 선언

최종 eligibility는 사전 전환 status guard와 신뢰된 active-record transition 후에 직접 기록하지 않고 파생해야 합니다. 닫힌 상태와 각 상태를 결정하는 조건은 다음과 같습니다.

- `invalid`, `suspect`, `stale`: `methexis.status.demotion-evidence`가 제공하는 해당 winning condition
- `inactive`: ineligible guard condition이 없고 신뢰된 active Checkpoint가 revision을 선택하지 않음
- `active`: ineligible guard condition이 없고 신뢰된 active Checkpoint가 revision을 선택함

최종 우선순위는 guard 순서와 membership 순서를 유지하여 `invalid > suspect > stale > inactive > active`여야 합니다.

모든 최종 eligibility 상태는 winning condition의 machine-readable evidence를 포함해야 합니다. `invalid`, `suspect`, `stale`는 winning guard evidence를 보존해야 합니다. `inactive`와 `active`는 정확한 신뢰된 active Checkpoint와 그 Checkpoint가 revision을 제외했는지 선택했는지를 식별해야 합니다.

일반 context에는 approval이 `approved`이고 eligibility가 `active`인 지식만 들어가야 합니다. 그 밖의 모든 조합은 제외해야 합니다. suspect와 stale content는 표시가 있는 diagnostic view에 계속 보여야 하고 일반 context로 내보내면 안 됩니다. invalid content는 context로 내보내면 안 됩니다.
