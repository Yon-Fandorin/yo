# Methexis Context

Canonical approved and active knowledge for this task. Treat `MUST` and `MUST NOT` as binding.

## KnowledgeUnit `tui.crate.ui-only-boundary`

# UI-only crate boundary

## Statement

The first `yo-tui` production crate MUST own only UI behavior, expose a narrow
facade, and keep implementation details internally visible by default.

## Rationale

A UI-only boundary keeps application and product semantics independent from
terminal presentation while avoiding speculative crate splits.
