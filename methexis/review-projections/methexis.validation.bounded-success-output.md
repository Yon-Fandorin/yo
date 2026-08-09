---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.validation.bounded-success-output
revision: sha256:7889d6f0d2242fa6e0ab5126a10b3fc23bc60996339b1ad6b8ff8e5a484c5b9f
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:4bf8f23522452289d7dff64cd15b528e8588905f277887e96948d703a1b106d5
---
# Korean Review Projection

## Translation

agent는 `--summary`로 크기가 제한된 성공 출력을 요청할 수 있습니다. summary는 전체 Knowledge 목록을 생략하되 requested class, executed class, 각 결과, authority, affected ID, diagnostic count를 유지해야 합니다.

`--unit <knowledge-id>`는 정확히 한 번만 사용할 수 있고 summary 출력이 켜져 있으며 요청에 `authority` 또는 `artifacts` class가 포함된 경우에만 허용됩니다. 검증이 그 authority-capable 단계까지 성공적으로 도달한 뒤에는 알려진 unit 선택이 정확히 그 unit 하나만 반환해야 하고, 알 수 없는 ID는 빈 성공이 아니라 usage failure여야 합니다. `--unit`을 두 번 이상 지정하거나 호환되지 않는 selector 조합은 검증 전에 usage failure여야 합니다.

출력 제한이 실패 증거를 숨기면 안 됩니다. 기반 검증 실패는 unit resolution보다 우선하며 summary 또는 unit selector와 관계없이 완전한 일반 보고서와 진단을 반환해야 합니다.
