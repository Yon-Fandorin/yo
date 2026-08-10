---
schema: methexis.knowledge/v1alpha1
id: methexis.architecture.process-boundaries
kind: rule
owner: methexis
sources:
  - id: methexis.architecture-model.process-boundaries
    revision: sha256:3b9da07bff00e21cd9f322f9a4249b3845502ea911d7b41b0b4b4ae0f5d16fc8
---
# Rust crate and process boundaries

## Statement

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
