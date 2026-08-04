---
schema: methexis.review-projection/v1alpha1
knowledge_id: tui.prompt-assist.trigger-lifecycle
revision: sha256:a7a8721bee4e81a1f20f25c4abee067a3fcfffef2bdd88e0e142739298440fb9
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:83fd7dc5f0e965f92749855b6e8471d4417ecd04dc91ec70e4294bd31db11b2b
---
# Korean Review Projection

## Translation

하나의 scanner와 controller가 Chat draft의 cursor 위치에서 `@` workspace reference와 `$` skill trigger 중 하나만 찾습니다. trigger는 draft 시작이나 Unicode whitespace 뒤에서만 시작하고, accepted annotation과 겹치거나 끝 경계에 닿은 raw trigger는 다시 열지 않습니다.

provider request와 update는 request identity와 immutable draft snapshot을 가지며 sequence가 증가해 terminal state 하나로 끝납니다. stale, cancelled, final 이후 update는 새 overlay나 draft를 바꾸지 못합니다. search는 agent backpressure와 독립이고 queued query는 최신 revision으로 합칠 수 있습니다.

editor mutation은 provider 결과를 기다리지 않고 즉시 화면에 반영되어야 합니다. 이미 보이는 trigger를 더 입력하면 기존 panel instance, 최근 usable entries, selection, styling을 그대로 유지합니다. destination geometry가 같으면 panel geometry와 viewport도 유지하지만 terminal resize에서는 selection panel의 일반 fitting과 hiding을 적용합니다. entry availability와 별개인 snapshot-level pending gate가 현재 draft와 일치하는 update 전까지 acceptance receipt를 막습니다. 이때 Tab과 Enter는 선택이나 제출 없이 소비합니다.

보이는 enabled result는 fresh 상태에서 Tab 또는 Enter로 accept하며 draft 제출은 다음 Enter에서 이루어집니다. accept는 전체 trigger token을 원자적으로 바꾸고 typed identity를 연결합니다. Esc는 현재 menu만 닫고 Ctrl+C와 global lifecycle 우선순위는 prompt-slot 계약을 따릅니다.
