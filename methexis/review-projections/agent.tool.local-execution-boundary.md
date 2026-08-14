---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.tool.local-execution-boundary
revision: sha256:9d93b13ce0cf5967fa567c9cbf9f73750c19a52a89e32479a9a119a5bd5a6dd8
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:bb658c5297ac8d2f2bcbcaa35f6e6efb8f85e017081d1526650c932d98fb8de2
---
# Korean Review Projection

## Translation

# Local model-tool 실행 경계

## 계약

Yo-managed model loop는 frontend-independent registry가 admit한 tool만 노출해야 합니다. 각 registered tool은 stable `ToolId`, unique wire name, safe description, versioned JSON input schema, typed effect와 approval requirement, injected execution-host handle을 가져야 합니다. Registry와 admission policy는 `yo-core`가 소유하고 execution host는 실제 OS 또는 remote workspace effect를 소유하며 그 effect를 Model Connector에 노출하면 안 됩니다.

Effective tool registry는 model request 하나 동안 고정해야 합니다. Model에는 admitted function-tool projection만 보내야 합니다. Provider built-in tool, provider-hosted code execution, direct provider MCP execution은 미루며 OpenAI-compatible endpoint 때문에 암묵적으로 enable하면 안 됩니다.

반환된 function call은 approval이나 execution 전에 exact registered tool 하나를 resolve하고 complete accumulated JSON argument를 validate해야 합니다. Invalid JSON, schema mismatch, unknown 또는 duplicate call identity, unavailable tool, configured argument bound 초과는 effect를 dispatch하지 않고 typed Tool Activity failure가 되어야 합니다. Approval은 exact Turn, call identity, ToolId, normalized argument digest, effect class, execution host에 bind해야 합니다. Stale 또는 mismatched response는 call을 authorize하면 안 됩니다.

Dispatch 뒤에는 call 하나당 local execution attempt를 최대 하나만 허용해야 합니다. Timeout, transport ambiguity, cancellation, executor failure, lost output 때문에 effectful tool을 자동 반복하면 안 됩니다. Executor는 bounded text output과 필요한 explicit truncation을 포함한 typed completed, failed, interrupted result를 반환해야 합니다. Session Journal이 exact call, approval, execution attempt, tool result를 correlate한 뒤에만 model에 result를 제출할 수 있습니다.

Execution progress와 absolute work budget은 서로 구분되어야 합니다. 모든 execution host는 유한한 progress-inactivity deadline과 이를 reset하는 정확한 signal을 정의해야 합니다. 첫 local command tool은 각 non-empty stdout 또는 stderr chunk를 progress로 취급하고 해당 progress마다 5분 inactivity window를 reset해야 하며, process가 살아 있어도 5분 동안 이런 output이 없으면 attempt를 실패시켜야 합니다. Agent policy는 absolute execution deadline 하나를 추가로 제공할 수 있습니다. 이 deadline은 기본값이 없음이고 attempt 하나에 대해 한 번 시작하며 output이나 다른 progress로 reset되면 안 됩니다. Cancellation은 두 wait를 모두 중단해야 하며 timeout 또는 cancellation은 유한한 termination, reap, output-drain bound를 사용해야 합니다. Diagnostic은 inactivity, agent가 제공한 absolute deadline, cancellation, cleanup failure를 구분해야 합니다. 어느 결과도 자동 retry를 허용하지 않습니다.

기본은 model order의 serial execution입니다. Scheduler가 approval scope와 mutable resource lease가 disjoint임을 증명할 때만 concurrent execution할 수 있습니다. Completion order와 관계없이 result publication과 model submission은 stable model-call order를 사용해야 합니다. Cancellation은 undispatched call을 막고 active executor의 prompt cancellation을 요청하며 effect가 없었다고 증명할 수 없으면 explicit interrupted result를 보존해야 합니다.

Tool name, schema, argument, output은 model-visible semantic history이며 Session Journal의 bounded persistence와 redaction을 따라야 합니다. Execution-host diagnostic과 prohibited secret은 semantic history 밖에 둬야 합니다. Exact replay는 historical tool을 다시 실행하지 않고 recorded function-call과 result relation을 재현해야 합니다.


첫 registry schema dialect는 closed `yo.tool-schema/v1` subset입니다. 각 node는 object, array, string, number, integer, boolean, null 중 하나가 필요하며 `description`, `properties`, `required`, `additionalProperties`, `items`, 같은 type의 non-empty `enum`만 허용합니다. Object schema는 `additionalProperties: false`를 설정해야 하고 array에는 item schema 하나가 필요하며 required name은 unique declared property여야 합니다. Unsupported keyword와 16단계 초과 schema 또는 instance nesting은 fail closed해야 합니다.

각 validation class는 diagnostic prose와 별도의 stable non-null `yo.tool.validation.*/v1` failure code를 제공해야 합니다. Dispatch 전 raw validated argument는 injected semantic-admission gate를 통과해야 합니다. Tool output은 Activity, 이후 model input, replay가 되기 전에 같은 gate를 통과해야 합니다. Gate는 exact admission, explicit bounded redacted replacement, Turn failure 중 하나를 반환할 수 있습니다. Credential, complete environment value, execution-host diagnostic, configured prohibited literal은 이 경계를 넘으면 안 되며 concrete tool은 gate를 우회할 수 없습니다. Concrete gate가 설치되기 전에는 native model에 local tool registry를 노출하면 안 됩니다.

## 이유

Delegated backend는 다른 agent host 안에 tool policy를 숨깁니다. Native loop에는 model protocol이 approval을 우회하거나 side effect를 반복하거나 tool completion order를 semantic order로 오인하지 못하게 하는 explicit local boundary가 필요합니다. Output inactivity와 선택적인 agent-owned absolute deadline을 분리하면 생산적으로 오래 실행되는 command를 허용하면서 silent stall을 탐지하고 cancellation을 보존할 수 있습니다.
