# Recurring workflow problems

Read only a matching row. These are navigation hints tied to executable owners,
not new gates. Recheck a recipe when its linked owner changes; remove it if the
failure no longer exists. Add a row only when it saves a repeatable lookup.

| Symptom | Inspect | Useful verification | Invalidation trigger |
|---|---|---|---|
| Ordinary commit unexpectedly demands Slice evidence | [ordinary preflight](../tools/xtask/src/impact/change.rs), [hook selection](../hk.pkl) | `cargo test -p xtask --test change_commit` | Hook selection or optional-claim semantics change |
| Formal Slice resumes with stale evidence or a rewritten candidate | [status](../tools/xtask/src/slice_status/mod.rs) | `cargo xtask slice status <slice>` and its returned next action | Evidence-chain or status protocol changes |
| A translated Developer Docs page is rejected after an English edit | [translation owner](../tools/xtask/src/docs_translation.rs) | Review matching Korean page, then `cargo xtask docs accept-translation <page>` | Canonical page meaning or projection protocol changes |
| Task context retrieval is empty or misleading | [route reader](../tools/context.py) | Search the exact symbol with `rg`; treat route output as incomplete | Route vocabulary, links, or query matching changes |
