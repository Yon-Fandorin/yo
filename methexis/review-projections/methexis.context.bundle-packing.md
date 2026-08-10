---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.context.bundle-packing
revision: sha256:6960889f6d3af2b68728ecf00c45cee6af8407f42e97c9a5c562c126f94bdd4e
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:22fb14dc3ce79889e1b306bd56e4367100799feb24af80459e2901506d2065fc
---
# Korean Review Projection

## Translation

root/candidate와 required closure는 하나의 원자적 bundle로 포함하거나 제외합니다. 필수 bundle을 먼저, 선택 후보를 Librarian 순서대로 greedy packing하며 실제 tokenizer 토큰을 계산하고 silent truncation이나 knapsack/LLM 최적화를 사용하지 않습니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

Selection operates on atomic semantic bundles. A root or candidate and its full
transitive `depends_on` and `constrained_by` closure are either included
together or not included. Shared required units are included and charged once.
A blocked or unaffordable required-root bundle fails the build. A blocked or
unaffordable optional-candidate bundle is omitted as a whole with a structured
manifest reason.

Packing uses deterministic greedy order. Required-root bundles are admitted
first. Optional candidates are then considered in the validated Librarian
order; a bundle is included when its marginal token cost fits, otherwise it is
omitted and later candidates are still considered. The Pilot does not use
score-per-token optimization, knapsack selection, an LLM reranker, or silent
body truncation.

The request names a supported versioned tokenizer profile and a maximum token
budget. The resolver counts the actual tokens of every byte-bearing element in
the final agent payload, including its preamble, stable IDs, headings, bodies,
and emitted relation text. It MUST NOT substitute character or byte estimates.
An unsupported profile is a structured failure. Tokenizer identity and version
are lineage inputs and change the BuildId. The first implementation supports
one profile and pins `o200k_base/v1` to `tiktoken-rs` 0.12.0.

Applicable existing validation evidence is recorded and attached when the
corpus provides it. In the first compiler profile it appears only in the
manifest and consumes no agent-payload tokens. Context resolution does not
execute validation commands, invent missing evidence, or describe a
`validated_by` reference as an executed result. Evidence execution and
collection are a later capability.
