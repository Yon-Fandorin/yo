---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.architecture.process-boundaries
revision: sha256:76c8d04164727c7a68e3a981d351f795bb3fb7c4b205c13d5fdedd594ddbbe9e
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:f7b48992755dc52e8e3bfea9db4e39fde17b9bb62a0ab45708d9b315365064bb
---
# Korean Review Projection

## Translation

Pilot는 tools/methexis와 tools/librarian 두 crate만 두고 각 crate는 library와 thin binary 하나를 가집니다. versioned JSON으로 교환하고 shared contract crate나 별도 resolver/service/database를 추가하지 않으며 졸업 후 yo에는 얇은 adapter만 남깁니다.

### 원문 대조

아래 내용은 기존에 승인된 SOT Pilot 정본에서 의미 변경 없이 옮긴 canonical English 본문입니다. 규범 키워드, 식별자, 예외 및 경계까지 이 원문을 기준으로 검토합니다.

After the repository foundation establishes the root Cargo workspace, add
exactly two initial tool crates:

```text
tools/methexis
tools/librarian
```

Each crate contains one library and one thin binary. Internal concerns remain
modules until an independent consumer justifies another crate.

The tools exchange a versioned candidate JSON artifact. Methexis MUST validate
that artifact and MUST NOT depend on Librarian's internal Rust types. Do not add
a shared contract crate in the Pilot.

The first ContextBuild implementation remains inside the existing Methexis
library and thin binary. It MUST NOT add a resolver crate, database, background
service, external connector, HTML view, GUI, or evidence runner. Those remain
separate evidence-gated capabilities.

This split follows lifecycle rather than module count. Both tools incubate in
`yo` and are expected to graduate to standalone repositories. After each
graduation, `yo` retains a thin adapter, reference corpus, contract fixtures,
and integration evaluation rather than a second implementation.
