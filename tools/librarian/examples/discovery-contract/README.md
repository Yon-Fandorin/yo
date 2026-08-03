# Librarian discovery contract examples

Run a request from the repository root:

```console
cargo run --quiet --locked -p librarian -- discover \
  --repository tools/librarian/examples/discovery-contract/corpus \
  tools/librarian/examples/discovery-contract/query-english.json
```

The command emits one complete candidate-set JSON value to stdout. Redirect it
with normal shell composition when a file is useful:

```console
cargo run --quiet --locked -p librarian -- discover \
  --repository tools/librarian/examples/discovery-contract/corpus \
  tools/librarian/examples/discovery-contract/query-english.json > candidates.json
```

The examples exercise canonical English, an exact-revision Korean Projection,
exact KnowledgeId and `applies_to` anchors, successful no-match discovery, and
a successful unresolved anchor. They are contract inputs, not persistent
Librarian-owned candidate artifacts.

`expected-query-english.json` and
`expected-failure-duplicate-anchor.json` are complete golden wire outputs.
Tests compare them byte-for-byte with stdout and stderr respectively; they make
schema, hashes, reason shapes, ordering, and failure isolation inspectable.

`corpus/` is a deliberately small, frozen protocol fixture. It is not Methexis
authority and must not grow with the live repository SOT. Its one real
Knowledge ID and revision let Methexis consume the Librarian golden while
unrelated SOT additions leave this wire contract unchanged.
