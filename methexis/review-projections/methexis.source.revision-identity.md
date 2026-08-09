---
schema: methexis.review-projection/v1alpha1
knowledge_id: methexis.source.revision-identity
revision: sha256:e69e99b82d6f2cb693938f4347ec368deb95a95c2589fc18eafe8396d85f4032
profile: ko-review/v1alpha1
compiler: methexis/0.0.0
request_hash: sha256:83cdca052c0c692e70fd20877498a3f6f01f67d1d2883573d8f5b9a74e3cb4c5
---
# Korean Review Projection

## Translation

# Source revision identity

## 선언

`SourceRevision`은 Source schema, SourceId, kind, 해당 kind의 모든 의미 field를 포함하는 하나의 domain-separated, length-delimited 표현에 대한 `sha256:<lowercase-hex>`로 인코딩해야 합니다.

YAML formatting, 물리 record path, generation time, code line hint, record의 revision field는 SourceRevision에 영향을 주면 안 됩니다. code path, symbol, content hash는 의미 정보이므로 영향을 줘야 합니다. Decision과 허가된 excerpt의 content, opaque reference와 hash, 각 external freshness mode의 locator 또는 reference, hash, version, expiry field는 해당 payload에 존재할 때 revision에 영향을 줘야 합니다.
