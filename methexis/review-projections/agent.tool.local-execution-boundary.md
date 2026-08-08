---
schema: methexis.review-projection/v1alpha1
knowledge_id: agent.tool.local-execution-boundary
revision: sha256:008b8c0d5eae8069bfc67404de3fe9eb8cb64d3fd385bd445fab6a3b2ebb2914
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:7b39222c29d0e3f907f94bd765c143b7dc0d76d217bccbf7dc081c83080a9403
---
# Korean Review Projection

## Translation

# Local model-tool 실행 경계

## 계약

Yo-managed model loop는 frontend-independent registry가 admit한 tool만 노출합니다. 각 tool은 stable ToolId, unique wire name, safe description, versioned JSON input schema, typed effect와 approval requirement, injected execution-host handle을 가집니다. Registry와 admission policy는 yo-core가 소유하고 execution host는 실제 OS 또는 remote workspace effect를 소유하며 그 effect를 Model Connector에 노출하지 않습니다.

Effective registry는 model request 하나 동안 고정합니다. Model에는 admitted function-tool projection만 보냅니다. Provider built-in tool, provider-hosted code execution, direct provider MCP execution은 미루며 OpenAI-compatible endpoint 때문에 암묵적으로 enable하지 않습니다.

Function call은 approval이나 execution 전에 exact registered tool 하나를 resolve하고 complete accumulated JSON arguments를 validate합니다. Invalid JSON, schema mismatch, unknown 또는 duplicate call identity, unavailable tool, argument bound 초과는 effect를 dispatch하지 않고 typed Tool Activity failure가 됩니다. Approval은 exact Turn, call identity, ToolId, normalized argument digest, effect class, execution host에 bind합니다. Stale 또는 mismatched response는 call을 authorize하지 않습니다.

Dispatch 뒤에는 local execution attempt를 하나만 허용합니다. Timeout, transport ambiguity, cancellation, executor failure, lost output 때문에 effectful tool을 자동 반복하지 않습니다. Executor는 bounded text output과 필요한 explicit truncation을 포함한 completed, failed, interrupted result를 반환합니다. Session Journal이 exact call, approval, attempt, result를 correlate한 뒤에만 model에 result를 제출할 수 있습니다.

기본은 model order의 serial execution입니다. Scheduler가 approval scope와 mutable resource lease가 disjoint임을 증명할 때만 concurrent execution할 수 있습니다. Completion order와 관계없이 result publication과 model submission은 stable model-call order를 사용합니다. Cancellation은 undispatched call을 막고 active executor의 prompt cancellation을 요청하며 effect가 없었다고 증명할 수 없으면 explicit interrupted result를 보존합니다.

Tool name, schema, arguments, output은 model-visible semantic history이며 Session Journal의 bounded persistence와 redaction을 따릅니다. Execution-host diagnostic과 prohibited secret은 semantic history 밖에 둡니다. Exact replay는 historical tool을 다시 실행하지 않고 recorded function-call과 result relation을 재현합니다.


첫 registry schema dialect는 closed yo.tool-schema/v1 subset입니다. 각 node는 object, array, string, number, integer, boolean, null 중 하나이며 description, properties, required, additionalProperties, items, 같은 type의 non-empty enum만 허용합니다. Object는 additionalProperties: false가 필수이고 array는 item schema가 필요하며 required는 unique declared property만 지칭합니다. Unsupported keyword와 16단계 초과 nesting은 fail closed합니다. 각 validation class는 diagnostic prose와 별도의 stable non-null yo.tool.validation.*/v1 code를 제공합니다. Argument는 dispatch 전에, output은 Activity·후속 model input·replay 전에 injected semantic-admission gate를 통과합니다. Gate는 exact admission, explicit bounded redacted replacement, 또는 Turn failure만 반환합니다. Credential, complete environment value, execution-host diagnostic, configured prohibited literal은 이 경계를 넘지 않으며 concrete tool은 gate를 우회할 수 없습니다. Gate가 설치되기 전에는 native model에 local tool registry를 노출하지 않습니다.

## 이유

Delegated backend는 다른 agent host 안에 tool policy를 숨기지만 native loop는 explicit local boundary가 필요합니다. 이 경계는 model protocol이 approval을 우회하거나 side effect를 반복하거나 completion order를 semantic order로 오인하지 못하게 합니다.
